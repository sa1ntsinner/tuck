//! The Windows side: who is in front, where monitors are, forcing focus,
//! and one background thread with the mouse hook (hold the middle button)
//! and the window-event hook (stickies that follow an app).
//!
//! The middle-button hold swallows the press and waits. Released early or
//! moved away, the press is replayed so autoscroll and "open in new tab"
//! keep working. Screen edges are left to other tools and never touched.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::{DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::PWSTR;

pub fn hwnd(raw: isize) -> HWND {
    HWND(raw as *mut _)
}

pub fn exe_of(h: HWND) -> String {
    path_of(h).rsplit('\\').next().unwrap_or_default().to_lowercase()
}

/// The name an app gives itself (Helium ships as chrome.exe but says "Helium").
pub fn app_name(raw: isize) -> String {
    use windows::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
    let path = path_of(hwnd(raw));
    let exe = path.rsplit('\\').next().unwrap_or_default().to_lowercase();
    let from_file = (|| unsafe {
        let wide: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
        let size = GetFileVersionInfoSizeW(windows::core::PCWSTR(wide.as_ptr()), None);
        if size == 0 {
            return None;
        }
        let mut data = vec![0u8; size as usize];
        GetFileVersionInfoW(windows::core::PCWSTR(wide.as_ptr()), None, size, data.as_mut_ptr() as _).ok()?;
        let mut ptr = std::ptr::null_mut();
        let mut len = 0u32;
        let query: Vec<u16> = "\\VarFileInfo\\Translation".encode_utf16().chain(Some(0)).collect();
        if !VerQueryValueW(data.as_ptr() as _, windows::core::PCWSTR(query.as_ptr()), &mut ptr, &mut len).as_bool() || len < 4 {
            return None;
        }
        let pair = std::slice::from_raw_parts(ptr as *const u16, 2);
        let key = format!("\\StringFileInfo\\{:04x}{:04x}\\FileDescription", pair[0], pair[1]);
        let query: Vec<u16> = key.encode_utf16().chain(Some(0)).collect();
        if !VerQueryValueW(data.as_ptr() as _, windows::core::PCWSTR(query.as_ptr()), &mut ptr, &mut len).as_bool() || len == 0 {
            return None;
        }
        let text = String::from_utf16_lossy(std::slice::from_raw_parts(ptr as *const u16, len as usize - 1)).trim().to_string();
        (!text.is_empty() && text.len() < 40).then_some(text)
    })();
    let generic = |t: &str| matches!(t, "Windows Explorer" | "Application Frame Host") || t.ends_with(".exe");
    match from_file {
        Some(t) if !generic(&t) => t.trim_start_matches("Google ").trim_start_matches("Microsoft ").trim_start_matches("Windows ").trim_end_matches(" Host").to_string(),
        _ => friendly(&exe),
    }
}

fn path_of(h: HWND) -> String {
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(h, Some(&mut pid));
        let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return String::new() };
        let mut buf = [0u16; 520];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = CloseHandle(process);
        if !ok {
            return String::new();
        }
        String::from_utf16_lossy(&buf[..len as usize])
    }
}

pub fn title_of(h: HWND) -> String {
    let mut buf = [0u16; 512];
    let n = unsafe { GetWindowTextW(h, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

pub struct Front {
    pub hwnd: isize,
    pub exe: String,
    pub title: String,
}

pub fn front() -> Front {
    let h = unsafe { GetForegroundWindow() };
    Front { hwnd: h.0 as isize, exe: exe_of(h), title: title_of(h) }
}

/// A human name for an executable.
pub fn friendly(exe: &str) -> String {
    let base = exe.trim_end_matches(".exe");
    let known = [
        ("code", "VS Code"),
        ("cursor", "Cursor"),
        ("zed", "Zed"),
        ("devenv", "Visual Studio"),
        ("chrome", "Chrome"),
        ("msedge", "Edge"),
        ("firefox", "Firefox"),
        ("brave", "Brave"),
        ("arc", "Arc"),
        ("helium", "Helium"),
        ("obsidian", "Obsidian"),
        ("notion", "Notion"),
        ("windowsterminal", "Terminal"),
        ("explorer", "Explorer"),
        ("ms-teams", "Teams"),
        ("teams", "Teams"),
        ("zoom", "Zoom"),
        ("slack", "Slack"),
        ("discord", "Discord"),
        ("telegram", "Telegram"),
        ("outlook", "Outlook"),
        ("olk", "Outlook"),
        ("winword", "Word"),
        ("excel", "Excel"),
        ("powerpnt", "PowerPoint"),
        ("figma", "Figma"),
        ("siemens.automation.portal", "TIA Portal"),
    ];
    known.iter().find(|(k, _)| *k == base).map(|(_, v)| v.to_string()).unwrap_or_else(|| {
        let mut c = base.chars();
        c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
    })
}

pub fn cursor() -> (i32, i32) {
    let mut p = POINT::default();
    let _ = unsafe { GetCursorPos(&mut p) };
    (p.x, p.y)
}

fn monitor_info(m: HMONITOR) -> MONITORINFO {
    let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    let _ = unsafe { GetMonitorInfoW(m, &mut mi) };
    mi
}

/// Work area (without the taskbar) of the monitor holding the point.
pub fn work_area_at(x: i32, y: i32) -> RECT {
    monitor_info(unsafe { MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST) }).rcWork
}

pub fn work_area_of(raw: isize) -> RECT {
    monitor_info(unsafe { MonitorFromWindow(hwnd(raw), MONITOR_DEFAULTTONEAREST) }).rcWork
}

/// The visible frame of a window, without the invisible resize border.
pub fn rect_of(raw: isize) -> Option<RECT> {
    let h = hwnd(raw);
    unsafe {
        if !IsWindow(Some(h)).as_bool() {
            return None;
        }
        let mut r = RECT::default();
        if DwmGetWindowAttribute(h, DWMWA_EXTENDED_FRAME_BOUNDS, &mut r as *mut _ as _, size_of::<RECT>() as u32).is_ok() {
            return Some(r);
        }
        GetWindowRect(h, &mut r).ok().map(|_| r)
    }
}

pub fn is_minimized(raw: isize) -> bool {
    unsafe { IsIconic(hwnd(raw)).as_bool() }
}

/// A game or video in front, filling its monitor: stay out of the way.
pub fn front_is_fullscreen() -> bool {
    unsafe {
        let h = GetForegroundWindow();
        if h.0.is_null() || h == GetShellWindow() || h == GetDesktopWindow() {
            return false;
        }
        let mut class = [0u16; 64];
        let n = GetClassNameW(h, &mut class);
        let class = String::from_utf16_lossy(&class[..n.max(0) as usize]);
        if class == "WorkerW" || class == "Progman" {
            return false;
        }
        let mut r = RECT::default();
        if GetWindowRect(h, &mut r).is_err() {
            return false;
        }
        let m = monitor_info(MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST)).rcMonitor;
        r.left <= m.left && r.top <= m.top && r.right >= m.right && r.bottom >= m.bottom
    }
}

/// Windows refuses focus to background apps; borrowing the foreground
/// thread's input state for a moment is the accepted way around it.
pub fn activate(raw: isize) {
    let target = hwnd(raw);
    unsafe {
        let front = GetForegroundWindow();
        let front_thread = GetWindowThreadProcessId(front, None);
        let me = GetCurrentThreadId();
        let attach = front_thread != 0 && front_thread != me;
        if attach {
            let _ = AttachThreadInput(me, front_thread, true);
        }
        let _ = ShowWindow(target, SW_SHOW);
        let _ = SetForegroundWindow(target);
        let _ = BringWindowToTop(target);
        let _ = SetFocus(Some(target));
        if attach {
            let _ = AttachThreadInput(me, front_thread, false);
        }
    }
}

/// Hand focus back to the window the person came from.
pub fn give_back(raw: isize) {
    let h = hwnd(raw);
    unsafe {
        if raw != 0 && IsWindow(Some(h)).as_bool() {
            let _ = SetForegroundWindow(h);
        }
    }
}

pub fn show_quiet(raw: isize) {
    unsafe {
        let _ = ShowWindow(hwnd(raw), SW_SHOWNOACTIVATE);
    }
}

pub fn is_visible(raw: isize) -> bool {
    unsafe { IsWindowVisible(hwnd(raw)).as_bool() }
}

pub fn hide(raw: isize) {
    unsafe {
        let _ = ShowWindow(hwnd(raw), SW_HIDE);
    }
}

/// "topmost" floats; otherwise the window goes under everything, like a note on the desk.
pub fn layer(raw: isize, topmost: bool) {
    let after = if topmost { HWND_TOPMOST } else { HWND_BOTTOM };
    unsafe {
        if !topmost {
            let _ = SetWindowPos(hwnd(raw), Some(HWND_NOTOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        }
        let _ = SetWindowPos(hwnd(raw), Some(after), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    }
}

/// Text on the clipboard, if any.
pub fn clipboard_text() -> Option<String> {
    use windows::Win32::System::DataExchange::*;
    use windows::Win32::System::Memory::{GlobalLock, GlobalUnlock};
    const CF_UNICODETEXT: u32 = 13;
    unsafe {
        if IsClipboardFormatAvailable(CF_UNICODETEXT).is_err() || OpenClipboard(None).is_err() {
            return None;
        }
        let mut out = None;
        if let Ok(handle) = GetClipboardData(CF_UNICODETEXT) {
            let global = HGLOBAL(handle.0);
            let p = GlobalLock(global) as *const u16;
            if !p.is_null() {
                let mut len = 0;
                while *p.add(len) != 0 && len < 200_000 {
                    len += 1;
                }
                out = Some(String::from_utf16_lossy(std::slice::from_raw_parts(p, len)));
                let _ = GlobalUnlock(global);
            }
        }
        let _ = CloseClipboard();
        out
    }
}

pub fn clipboard_age_ms() -> u64 {
    use windows::Win32::System::DataExchange::GetClipboardSequenceNumber;
    static LAST: Mutex<(u32, u64)> = Mutex::new((0, 0));
    let seq = unsafe { GetClipboardSequenceNumber() };
    let now = crate::store::now_ms() as u64;
    let mut last = LAST.lock().unwrap();
    if last.0 != seq {
        *last = (seq, now);
    }
    now.saturating_sub(last.1)
}

// ---------------------------------------------------------------- hooks

pub enum Hook {
    /// The middle button was held at this point.
    Hold(i32, i32),
    /// ...and let go.
    HoldEnd,
    Front(isize, String),
    Moved(isize),
}

static SINK: OnceLock<Box<dyn Fn(Hook) + Send + Sync>> = OnceLock::new();
static HOOK_THREAD: AtomicU32 = AtomicU32::new(0);
pub static HOLD_ON: AtomicBool = AtomicBool::new(true);
pub static HOLD_MS: AtomicU64 = AtomicU64::new(320);
pub static EXCLUDED: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// The window whose moves we report (the app a sticky follows).
pub static TRACKED: AtomicIsize = AtomicIsize::new(0);

const REPLAY_DOWN: u32 = WM_APP + 1;
const REPLAY_CLICK: u32 = WM_APP + 2;
const SLOP: i32 = 6;
const EDGE: i32 = 3;

#[derive(PartialEq)]
enum Phase {
    Idle,
    Waiting,
    Held,
}

struct HoldState {
    phase: Phase,
    at: POINT,
    timer: usize,
}

thread_local! {
    static HOLD: RefCell<HoldState> = const { RefCell::new(HoldState { phase: Phase::Idle, at: POINT { x: 0, y: 0 }, timer: 0 }) };
}

fn emit(h: Hook) {
    if let Some(sink) = SINK.get() {
        sink(h);
    }
}

fn near_edge(p: POINT) -> bool {
    let m = monitor_info(unsafe { MonitorFromPoint(p, MONITOR_DEFAULTTONEAREST) }).rcMonitor;
    p.x - m.left < EDGE || m.right - 1 - p.x < EDGE || p.y - m.top < EDGE || m.bottom - 1 - p.y < EDGE
}

fn wants(p: POINT) -> bool {
    if !HOLD_ON.load(Ordering::Relaxed) || near_edge(p) || front_is_fullscreen() {
        return false;
    }
    let under = unsafe { GetAncestor(WindowFromPoint(p), GA_ROOT) };
    let exe = exe_of(under);
    !EXCLUDED.lock().map(|list| list.iter().any(|e| e.eq_ignore_ascii_case(&exe))).unwrap_or(false)
}

fn post(msg: u32) {
    unsafe {
        let _ = PostThreadMessageW(HOOK_THREAD.load(Ordering::Relaxed), msg, WPARAM(0), LPARAM(0));
    }
}

fn replay(click: bool) {
    let input = |flags| INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 { mi: MOUSEINPUT { dwFlags: flags, ..Default::default() } },
    };
    let mut events = vec![input(MOUSEEVENTF_MIDDLEDOWN)];
    if click {
        events.push(input(MOUSEEVENTF_MIDDLEUP));
    }
    unsafe {
        SendInput(&events, size_of::<INPUT>() as i32);
    }
}

unsafe extern "system" fn mouse_proc(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code >= 0 {
        let info = unsafe { &*(lp.0 as *const MSLLHOOKSTRUCT) };
        if info.flags & LLMHF_INJECTED == 0 && HOLD.with_borrow_mut(|s| take(s, wp.0 as u32, info.pt)) {
            return LRESULT(1);
        }
    }
    unsafe { CallNextHookEx(None, code, wp, lp) }
}

/// True when the event is ours and must not reach the app.
fn take(s: &mut HoldState, msg: u32, pt: POINT) -> bool {
    match (msg, &s.phase) {
        (WM_MBUTTONDOWN, Phase::Idle) if wants(pt) => {
            s.phase = Phase::Waiting;
            s.at = pt;
            s.timer = unsafe { SetTimer(None, 0, HOLD_MS.load(Ordering::Relaxed) as u32, None) };
            true
        }
        (WM_MOUSEMOVE, Phase::Waiting) if (pt.x - s.at.x).abs() > SLOP || (pt.y - s.at.y).abs() > SLOP => {
            cancel(s);
            post(REPLAY_DOWN);
            false
        }
        (WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MOUSEWHEEL, Phase::Waiting) => {
            cancel(s);
            post(REPLAY_DOWN);
            false
        }
        (WM_MBUTTONUP, Phase::Waiting) => {
            cancel(s);
            post(REPLAY_CLICK);
            true
        }
        (WM_MBUTTONUP, Phase::Held) => {
            s.phase = Phase::Idle;
            emit(Hook::HoldEnd);
            true
        }
        _ => false,
    }
}

fn cancel(s: &mut HoldState) {
    unsafe {
        let _ = KillTimer(None, s.timer);
    }
    s.phase = Phase::Idle;
}

unsafe extern "system" fn event_proc(_: HWINEVENTHOOK, event: u32, h: HWND, object: i32, child: i32, _: u32, _: u32) {
    if object != OBJID_WINDOW.0 || child != 0 {
        return;
    }
    match event {
        EVENT_SYSTEM_FOREGROUND => emit(Hook::Front(h.0 as isize, exe_of(h))),
        EVENT_SYSTEM_MINIMIZESTART => emit(Hook::Front(0, String::new())),
        EVENT_OBJECT_LOCATIONCHANGE if h.0 as isize == TRACKED.load(Ordering::Relaxed) => emit(Hook::Moved(h.0 as isize)),
        _ => {}
    }
}

pub fn start_hooks(sink: impl Fn(Hook) + Send + Sync + 'static) {
    let _ = SINK.set(Box::new(sink));
    std::thread::spawn(|| unsafe {
        HOOK_THREAD.store(GetCurrentThreadId(), Ordering::Relaxed);
        let _mouse = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), None, 0);
        let flags = WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS;
        let _front = SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, None, Some(event_proc), 0, 0, flags);
        let _minimize = SetWinEventHook(EVENT_SYSTEM_MINIMIZESTART, EVENT_SYSTEM_MINIMIZESTART, None, Some(event_proc), 0, 0, flags);
        let _moves = SetWinEventHook(EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_LOCATIONCHANGE, None, Some(event_proc), 0, 0, flags);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            match msg.message {
                WM_TIMER if msg.hwnd.0.is_null() => {
                    let fired = HOLD.with_borrow_mut(|s| {
                        if s.timer != msg.wParam.0 {
                            return None;
                        }
                        let _ = KillTimer(None, s.timer);
                        (s.phase == Phase::Waiting).then(|| {
                            s.phase = Phase::Held;
                            s.at
                        })
                    });
                    if let Some(p) = fired {
                        emit(Hook::Hold(p.x, p.y));
                    }
                }
                REPLAY_DOWN => replay(false),
                REPLAY_CLICK => replay(true),
                _ => {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        }
    });
}
