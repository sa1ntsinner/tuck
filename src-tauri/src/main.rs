// A window app in every build: no console appears, even for a dev build started by Windows.
#![windows_subsystem = "windows"]

mod ai;
mod apps;
mod calls;
mod dest;
mod settings;
mod skill;
mod store;
mod win;

use serde::Serialize;
use settings::Settings;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use store::{Note, Pin};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

static APP: OnceLock<AppHandle> = OnceLock::new();
static SETTINGS: Mutex<Option<Settings>> = Mutex::new(None);
static CTX: Mutex<Option<Ctx>> = Mutex::new(None);
/// Pinned notes by id, so the window hooks never touch the disk.
static PINS: Mutex<Option<HashMap<String, Pin>>> = Mutex::new(None);
/// The window to hand focus back to, and the last outside window in front.
static PREV: AtomicIsize = AtomicIsize::new(0);
static FRONT: Mutex<(isize, String)> = Mutex::new((0, String::new()));
static HELD: AtomicBool = AtomicBool::new(false);
static KEYS: Mutex<Vec<(Shortcut, &'static str)>> = Mutex::new(Vec::new());
static SORTING: AtomicBool = AtomicBool::new(false);
static LAST_SORT: Mutex<Option<chrono::DateTime<chrono::Local>>> = Mutex::new(None);

#[derive(Serialize, Clone, Default)]
struct Ctx {
    /// "note", "sticky" or "call".
    mode: String,
    app: String,
    exe: String,
    title: String,
    clip: String,
    call: String,
    by_mouse: bool,
}

fn app() -> &'static AppHandle {
    APP.get().expect("app")
}

fn cfg() -> Settings {
    SETTINGS.lock().unwrap().clone().unwrap_or_default()
}

fn window(label: &str) -> Option<WebviewWindow> {
    app().get_webview_window(label)
}

fn raw(w: &WebviewWindow) -> isize {
    w.hwnd().map(|h| h.0 as isize).unwrap_or(0)
}

fn changed() {
    let _ = app().emit("notes", ());
}

fn on_main(f: impl FnOnce() + Send + 'static) {
    let _ = app().run_on_main_thread(f);
}

// ------------------------------------------------------------- capture

fn open_capture(mode: &str, at: Option<(i32, i32)>, call: &str) {
    let Some(w) = window("capture") else { return };
    if let Some(deck) = window("deck") {
        win::hide(raw(&deck));
    }
    let mut front = win::front();
    if front.exe == "tuck.exe" || front.hwnd == raw(&w) {
        let (h, exe) = FRONT.lock().unwrap().clone();
        front = win::Front { hwnd: h, title: win::title_of(win::hwnd(h)), exe };
    }
    PREV.store(front.hwnd, Ordering::Relaxed);
    let clip = if win::clipboard_age_ms() < 90_000 {
        win::clipboard_text().map(|t| t.trim().chars().take(4000).collect()).unwrap_or_default()
    } else {
        String::new()
    };
    let ctx = Ctx {
        mode: mode.into(),
        app: if front.exe.is_empty() { String::new() } else { win::app_name(front.hwnd) },
        exe: front.exe.clone(),
        title: front.title.clone(),
        clip,
        call: call.into(),
        by_mouse: at.is_some(),
    };
    *CTX.lock().unwrap() = Some(ctx.clone());

    let scale = w.scale_factor().unwrap_or(1.0);
    let s = cfg();
    let (lw, lh) = (s.note_w.clamp(420, 1600) as f64, s.note_h.clamp(130, 900) as f64);
    let _ = w.set_size(LogicalSize::new(lw, lh));
    let (pw, ph) = ((lw * scale) as i32, (lh.max(300.0) * scale) as i32);
    let (x, y) = match at {
        Some((cx, cy)) => {
            let r = win::work_area_at(cx, cy);
            ((cx - pw / 2).clamp(r.left + 8, r.right - pw - 8), (cy - (40.0 * scale) as i32).clamp(r.top + 8, r.bottom - ph - 8))
        }
        None => {
            let r = if front.hwnd != 0 { win::work_area_of(front.hwnd) } else { win::work_area_at(win::cursor().0, win::cursor().1) };
            (r.left + (r.right - r.left - pw) / 2, r.top + (r.bottom - r.top) * 22 / 100)
        }
    };
    let _ = w.set_position(PhysicalPosition::new(x, y));
    let _ = w.emit("capture", ctx);
    let h = raw(&w);
    on_main(move || win::activate(h));
}

fn dismiss_capture() {
    if let Some(w) = window("capture") {
        win::hide(raw(&w));
    }
    let prev = PREV.load(Ordering::Relaxed);
    on_main(move || win::give_back(prev));
}

// ---------------------------------------------------------------- deck

fn toggle_deck(force_open: bool) {
    let Some(w) = window("deck") else { return };
    if win::is_visible(raw(&w)) && !force_open {
        win::hide(raw(&w));
        return;
    }
    let (cx, cy) = win::cursor();
    let r = win::work_area_at(cx, cy);
    let scale = w.scale_factor().unwrap_or(1.0);
    let (pw, ph) = ((420.0 * scale) as i32, (640.0 * scale) as i32);
    let _ = w.set_position(PhysicalPosition::new(r.right - pw - (10.0 * scale) as i32, r.bottom - ph - (10.0 * scale) as i32));
    let _ = w.emit("deck", ());
    let h = raw(&w);
    on_main(move || win::activate(h));
}

// ------------------------------------------------------------- stickies

/// Window labels allow only a few characters; ids of hand-written notes can hold anything.
fn sticky_label(id: &str) -> String {
    let safe: String = id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    format!("sticky-{safe}")
}

fn remember_pins() {
    let map: HashMap<String, Pin> = store::list("deck").into_iter().filter_map(|n| Some((n.id, n.pin?))).collect();
    *PINS.lock().unwrap() = Some(map);
}

fn pins() -> HashMap<String, Pin> {
    PINS.lock().unwrap().clone().unwrap_or_default()
}

fn show_sticky(id: &str, pin: &Pin, focus: bool) {
    let label = sticky_label(id);
    let w = match window(&label) {
        Some(w) => w,
        None => {
            let url = WebviewUrl::App(format!("index.html?w=sticky&id={id}").into());
            let Ok(w) = WebviewWindowBuilder::new(app(), &label, url)
                .title("Tuck note")
                .decorations(false)
                .transparent(true)
                .shadow(false)
                .skip_taskbar(true)
                .resizable(false)
                .visible(false)
                .focused(false)
                .inner_size(pin.w.max(180.0), pin.h.max(120.0))
                .build()
            else {
                return;
            };
            w
        }
    };
    let h = raw(&w);
    match pin.mode.as_str() {
        "app" => follow(h, pin),
        mode => {
            let _ = w.set_position(PhysicalPosition::new(pin.x, pin.y));
            win::show_quiet(h);
            win::layer(h, mode != "desk");
        }
    }
    if focus {
        on_main(move || win::activate(h));
    }
}

/// Place an app-pinned sticky on its app, or hide it when the app isn't in front.
fn follow(h: isize, pin: &Pin) {
    let (front, exe) = FRONT.lock().unwrap().clone();
    match (exe == pin.app && !win::is_minimized(front)).then(|| win::rect_of(front)).flatten() {
        Some(r) => {
            unsafe_move(h, r.left + pin.x, r.top + pin.y);
            win::show_quiet(h);
            win::layer(h, true);
        }
        None => win::hide(h),
    }
}

fn unsafe_move(h: isize, x: i32, y: i32) {
    if let Some(w) = app().webview_windows().into_values().find(|w| raw(w) == h) {
        let _ = w.set_position(PhysicalPosition::new(x, y));
    }
}

fn close_sticky(id: &str) {
    if let Some(w) = window(&sticky_label(id)) {
        let _ = w.destroy();
    }
}

fn on_front(h: isize, exe: String) {
    if !exe.is_empty() && exe != "tuck.exe" {
        *FRONT.lock().unwrap() = (h, exe.clone());
    } else if h == 0 {
        *FRONT.lock().unwrap() = (0, String::new());
    }
    let mut track = 0;
    for (id, pin) in pins() {
        let Some(w) = window(&sticky_label(&id)) else { continue };
        match pin.mode.as_str() {
            "app" => {
                if pin.app == exe {
                    track = h;
                }
                follow(raw(&w), &pin);
            }
            "desk" => win::layer(raw(&w), false),
            _ => {}
        }
    }
    win::TRACKED.store(track, Ordering::Relaxed);
}

fn on_moved(h: isize) {
    let exe = FRONT.lock().unwrap().1.clone();
    for (id, pin) in pins() {
        if pin.mode == "app" && pin.app == exe {
            if let Some(w) = window(&sticky_label(&id)) {
                follow(raw(&w), &pin);
            }
        }
    }
    let _ = h;
}

fn pin_note(n: &mut Note, mode: &str, near: Option<(i32, i32)>) {
    let old = n.pin.clone().unwrap_or(Pin { w: 260.0, h: 220.0, zoom: 1.0, ..Default::default() });
    let mut pin = Pin { mode: mode.into(), ..old };
    let (cx, cy) = near.unwrap_or_else(win::cursor);
    if mode == "app" {
        let (front, exe) = FRONT.lock().unwrap().clone();
        let ctx_exe = CTX.lock().unwrap().as_ref().map(|c| c.exe.clone()).unwrap_or_default();
        pin.app = if ctx_exe.is_empty() { exe } else { ctx_exe };
        let r = win::rect_of(PREV.load(Ordering::Relaxed)).or_else(|| win::rect_of(front));
        if let Some(r) = r {
            pin.x = (r.right - r.left) - (pin.w * 1.5) as i32 - 24;
            pin.y = 56;
        }
    } else {
        let r = win::work_area_at(cx, cy);
        pin.x = (cx - 40).clamp(r.left, r.right - 300);
        pin.y = (cy - 20).clamp(r.top, r.bottom - 260);
    }
    n.pin = Some(pin);
}

// ------------------------------------------------------------ commands

#[tauri::command]
async fn ctx() -> Ctx {
    CTX.lock().unwrap().clone().unwrap_or_default()
}

#[tauri::command]
async fn notes(scope: String) -> Vec<Note> {
    store::list(if matches!(scope.as_str(), "sent" | "trash") { &scope } else { "deck" })
}

#[tauri::command]
async fn note(id: String) -> Option<Note> {
    store::find(&id).map(|(_, n)| n)
}

#[tauri::command]
async fn save(mut note: Note, action: String) -> Result<Note, String> {
    if note.id.is_empty() {
        note.id = store::new_id();
        note.created = store::now_ms();
    }
    if note.color.is_empty() {
        note.color = "butter".into();
    }
    let capture_pos = window("capture").and_then(|w| w.outer_position().ok()).map(|p| (p.x + 60, p.y + 30));
    match action.as_str() {
        "pin" => pin_note(&mut note, "float", capture_pos),
        "pin_app" => pin_note(&mut note, "app", capture_pos),
        _ => {}
    }
    store::write("deck", &note).map_err(|e| e.to_string())?;
    // Focus goes back at once; the panel leaves after its exit animation.
    // Hidden from here because a webview without focus may pause its timers.
    let prev = PREV.load(Ordering::Relaxed);
    on_main(move || win::give_back(prev));
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_millis(250));
        if let Some(w) = window("capture") {
            win::hide(raw(&w));
        }
    });
    if let Some(pin) = note.pin.clone() {
        remember_pins();
        show_sticky(&note.id, &pin, false);
    }
    if action == "send" {
        send(note.id.clone(), None).await?;
    }
    changed();
    Ok(note)
}

#[tauri::command]
async fn update(note: Note) -> Result<(), String> {
    let folder = store::find(&note.id).map(|(f, _)| f).unwrap_or_else(|| "deck".into());
    store::write(&folder, &note).map_err(|e| e.to_string())?;
    changed();
    Ok(())
}

#[tauri::command]
async fn remove(id: String) -> Result<(), String> {
    let (folder, n) = store::find(&id).ok_or("Note not found")?;
    if folder == "trash" {
        store::delete("trash", &id);
    } else {
        store::shift(&n, &folder, "trash").map_err(|e| e.to_string())?;
    }
    close_sticky(&id);
    remember_pins();
    changed();
    Ok(())
}

#[tauri::command]
async fn restore(id: String) -> Result<(), String> {
    let (folder, mut n) = store::find(&id).ok_or("Note not found")?;
    n.sent = None;
    n.sent_at = None;
    store::shift(&n, &folder, "deck").map_err(|e| e.to_string())?;
    remember_pins();
    if let Some(pin) = n.pin.clone() {
        show_sticky(&n.id, &pin, false);
    }
    changed();
    Ok(())
}

#[tauri::command]
async fn send(id: String, dest: Option<String>) -> Result<String, String> {
    let (folder, mut n) = store::find(&id).ok_or("Note not found")?;
    let s = cfg();
    let chosen = dest.or(n.to.clone()).and_then(|d| s.dests.iter().find(|x| x.id == d).cloned());
    let d = chosen.or_else(|| dest::route(&s, &n).cloned()).ok_or("Pick where notes go first: Settings → Send by hand to")?;
    let place = apps::deliver(&d, &n, s.keep_source)?;
    n.sent = Some(format!("{} · {place}", d.name));
    n.sent_at = Some(store::now_ms());
    n.pin = None;
    store::shift(&n, &folder, "sent").map_err(|e| e.to_string())?;
    close_sticky(&id);
    remember_pins();
    changed();
    Ok(d.name)
}

#[derive(Serialize)]
struct SendAll {
    sent: usize,
    kept: usize,
    errors: Vec<String>,
}

/// Sends every unpinned card that has somewhere to go.
#[tauri::command]
async fn send_all() -> SendAll {
    let s = cfg();
    let mut out = SendAll { sent: 0, kept: 0, errors: Vec::new() };
    for n in store::list("deck") {
        let routed = n.to.as_ref().map(|t| s.dests.iter().any(|d| &d.id == t)).unwrap_or(false) || dest::route(&s, &n).is_some();
        if n.pin.is_some() || !routed {
            out.kept += 1;
            continue;
        }
        match send(n.id.clone(), None).await {
            Ok(_) => out.sent += 1,
            Err(e) => {
                if !out.errors.contains(&e) {
                    out.errors.push(e);
                }
            }
        }
    }
    out
}

#[tauri::command]
async fn pin(id: String, mode: String) -> Result<(), String> {
    let (folder, mut n) = store::find(&id).ok_or("Note not found")?;
    if mode.is_empty() {
        n.pin = None;
        close_sticky(&id);
    } else {
        pin_note(&mut n, &mode, None);
        if let Some(w) = window(&sticky_label(&id)) {
            let _ = w.destroy();
        }
    }
    store::write(&folder, &n).map_err(|e| e.to_string())?;
    remember_pins();
    if let Some(p) = n.pin.clone() {
        show_sticky(&id, &p, false);
    }
    changed();
    Ok(())
}

/// The sticky was dragged, resized or zoomed: remember where it is.
#[tauri::command]
async fn sticky_geom(id: String, w: f64, h: f64, zoom: f64, color: Option<String>) -> Result<(), String> {
    let (folder, mut n) = store::find(&id).ok_or("Note not found")?;
    let Some(mut pin) = n.pin.clone() else { return Ok(()) };
    let window = window(&sticky_label(&id)).ok_or("No window")?;
    let pos = window.outer_position().map_err(|e| e.to_string())?;
    if pin.mode == "app" {
        let front = FRONT.lock().unwrap().0;
        if let Some(r) = win::rect_of(front) {
            pin.x = pos.x - r.left;
            pin.y = pos.y - r.top;
        }
    } else {
        pin.x = pos.x;
        pin.y = pos.y;
    }
    pin.w = w;
    pin.h = h;
    pin.zoom = zoom;
    n.pin = Some(pin);
    if let Some(c) = color {
        n.color = c;
    }
    store::write(&folder, &n).map_err(|e| e.to_string())?;
    remember_pins();
    Ok(())
}

#[tauri::command]
async fn get_settings() -> Settings {
    settings::masked(&cfg())
}

#[derive(Serialize)]
struct Applied {
    failed: Vec<String>,
}

#[tauri::command]
async fn set_settings(s: Settings) -> Result<Applied, String> {
    let old = cfg();
    let merged = settings::merge(&old, s);
    if merged.inbox.trim() != old.inbox.trim() {
        let from = store::root();
        store::set_root(&merged.inbox);
        if let Err(e) = store::relocate(&from, &store::root()) {
            store::set_root(&old.inbox);
            return Err(format!("Can't move the notes there: {e}"));
        }
        changed();
    }
    settings::save(&merged);
    *SETTINGS.lock().unwrap() = Some(merged.clone());
    if merged.skill_auto {
        let _ = skill::install(&merged);
    }
    Ok(Applied { failed: apply() })
}

/// Pushes settings into the hooks and the shortcuts; returns keys Windows refused.
fn apply() -> Vec<String> {
    let s = cfg();
    win::HOLD_ON.store(s.middle_hold, Ordering::Relaxed);
    win::HOLD_MS.store(s.hold_ms.clamp(180, 900), Ordering::Relaxed);
    *win::EXCLUDED.lock().unwrap() = s.excluded.clone();
    let gs = app().global_shortcut();
    let _ = gs.unregister_all();
    let mut keys = Vec::new();
    let mut failed = Vec::new();
    for (text, what) in [(&s.capture_key, "capture"), (&s.sticky_key, "sticky"), (&s.deck_key, "deck")] {
        if text.trim().is_empty() {
            continue;
        }
        match Shortcut::from_str(text) {
            Ok(sc) if gs.register(sc).is_ok() => keys.push((sc, what)),
            _ => failed.push(text.clone()),
        }
    }
    *KEYS.lock().unwrap() = keys;
    failed
}

#[tauri::command]
async fn vaults() -> Vec<dest::Vault> {
    dest::vaults()
}

#[tauri::command]
async fn found_apps() -> Vec<apps::Found> {
    tauri::async_runtime::spawn_blocking(apps::found).await.unwrap_or_default()
}

#[tauri::command]
async fn anytype_challenge() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(apps::anytype_challenge).await.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn notion_pages(token: String) -> Result<Vec<apps::Page>, String> {
    tauri::async_runtime::spawn_blocking(move || apps::notion_pages(&token)).await.map_err(|e| e.to_string())?
}

/// Connects an app and adds it to the places, so its token never goes back to the page.
/// `a` and `b` mean: joplin -, anytype challenge+code, todoist/tana token, notion token+page,
/// obsidian/logseq/folder path.
#[tauri::command]
async fn connect(kind: String, a: String, b: String) -> Result<String, String> {
    let k = kind.clone();
    let mut d = tauri::async_runtime::spawn_blocking(move || -> Result<settings::Dest, String> {
        match k.as_str() {
            "joplin" => apps::joplin_connect(),
            "anytype" => apps::anytype_connect(&a, &b),
            "todoist" | "tana" => apps::token_connect(&k, &a),
            "notion" => {
                let page = dest::notion_id(&b).ok_or("Pick a page")?;
                let title = apps::notion_pages(&a)?.into_iter().find(|p| p.id == page).map(|p| p.title).unwrap_or_else(|| "Notion".into());
                Ok(settings::Dest { kind: "notion".into(), name: title, page, token: settings::seal(a.trim()), ..Default::default() })
            }
            "obsidian" | "logseq" | "folder" => {
                let p = std::path::Path::new(a.trim());
                if !p.is_dir() {
                    return Err("That folder doesn't exist".into());
                }
                let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "Folder".into());
                Ok(settings::Dest { kind: k.clone(), name, path: a.trim().into(), mode: "daily".into(), ..Default::default() })
            }
            _ => Err("Unknown app".into()),
        }
    })
    .await
    .map_err(|e| e.to_string())??;
    d.id = store::new_id();
    if d.mode.is_empty() {
        d.mode = "daily".into();
    }
    let mut s = cfg();
    if s.default_dest.is_empty() || !s.dests.iter().any(|x| x.id == s.default_dest) {
        s.default_dest = d.id.clone();
    }
    let name = d.name.clone();
    s.dests.push(d);
    settings::save(&s);
    *SETTINGS.lock().unwrap() = Some(s);
    Ok(name)
}

#[tauri::command]
async fn sort_plan() -> Result<Vec<ai::Step>, String> {
    let s = cfg();
    let notes: Vec<Note> = store::list("deck").into_iter().filter(|n| n.pin.is_none()).collect();
    tauri::async_runtime::spawn_blocking(move || ai::plan(&s, &notes)).await.map_err(|e| e.to_string())?
}

#[derive(Serialize)]
struct Sorted {
    done: usize,
    errors: Vec<String>,
}

/// Writes each step into the library, then files the note as sent.
fn sort_steps(steps: &[ai::Step]) -> Sorted {
    // A manual Sort and the scheduled one must never write the same note twice.
    static ONE: Mutex<()> = Mutex::new(());
    let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
    let s = cfg();
    let mut out = Sorted { done: 0, errors: Vec::new() };
    let Some(lib) = ai::library(&s) else {
        out.errors.push("Choose the folder notes get sorted into".into());
        return out;
    };
    for st in steps.iter().filter(|st| st.action != "keep") {
        let Some((folder, mut n)) = store::find(&st.id) else { continue };
        if folder != "deck" || n.pin.is_some() {
            continue;
        }
        match ai::write(&lib, st) {
            Ok(place) => {
                n.sent = Some(place);
                n.sent_at = Some(store::now_ms());
                if store::shift(&n, "deck", "sent").is_ok() {
                    out.done += 1;
                }
            }
            Err(e) => out.errors.push(format!("{}: {e}", st.path)),
        }
    }
    changed();
    out
}

#[tauri::command]
async fn sort_apply(steps: Vec<ai::Step>) -> Sorted {
    tauri::async_runtime::spawn_blocking(move || sort_steps(&steps)).await.unwrap_or(Sorted { done: 0, errors: vec!["Sort stopped".into()] })
}

#[derive(Serialize)]
struct SortState {
    ready: bool,
    library: Option<String>,
    inbox: String,
    inbox_ok: bool,
    agents: Vec<String>,
}

#[tauri::command]
async fn sort_state() -> SortState {
    let s = cfg();
    SortState {
        ready: ai::configured(&s),
        library: ai::library(&s).map(|p| p.display().to_string()),
        inbox: store::root().display().to_string(),
        inbox_ok: store::inbox_ok(),
        agents: skill::agents(),
    }
}

#[tauri::command]
async fn providers() -> Vec<ai::Status> {
    tauri::async_runtime::spawn_blocking(ai::statuses).await.unwrap_or_default()
}

/// Lists models; a key of "•" means the one already saved.
#[tauri::command]
async fn models(provider: String, url: String, key: String) -> Result<Vec<String>, String> {
    let key = if key == "•" { settings::open(&cfg().ai_key) } else { key };
    tauri::async_runtime::spawn_blocking(move || ai::models(&provider, &url, &key)).await.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn openrouter_login() -> Result<(), String> {
    let key = tauri::async_runtime::spawn_blocking(ai::openrouter_login).await.map_err(|e| e.to_string())??;
    let mut s = cfg();
    s.ai_provider = "openrouter".into();
    s.ai_key = settings::seal(&key);
    settings::save(&s);
    *SETTINGS.lock().unwrap() = Some(s);
    Ok(())
}

#[tauri::command]
async fn cli_login(provider: String) -> Result<(), String> {
    ai::cli_login(&provider)
}

/// One tiny request with the saved settings, so setup can say "works".
#[tauri::command]
async fn test_ai() -> Result<(), String> {
    let s = cfg();
    tauri::async_runtime::spawn_blocking(move || ai::test(&s)).await.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn open_url(url: String) {
    if url.starts_with("https://") {
        ai::open_url(&url);
    }
}

#[tauri::command]
async fn dismiss() {
    dismiss_capture();
}

#[tauri::command]
async fn log(window: WebviewWindow, msg: String) {
    eprintln!("[{}] {msg}", window.label());
}

#[tauri::command]
async fn hide_window(window: WebviewWindow) {
    win::hide(raw(&window));
}

#[tauri::command]
async fn open_deck() {
    toggle_deck(true);
}

#[tauri::command]
async fn new_capture(mode: String) {
    open_capture(&mode, None, "");
}

#[tauri::command]
async fn reveal(what: String) {
    open_folder(&what);
}

fn open_folder(what: &str) {
    let path = if what == "settings" { std::path::PathBuf::from(std::env::var("APPDATA").unwrap_or_default()).join("Tuck") } else { store::root() };
    let _ = std::process::Command::new("explorer").arg(path).spawn();
}

#[tauri::command]
async fn install_skill() -> Result<Vec<String>, String> {
    skill::install(&cfg())
}

#[tauri::command]
async fn autostart(on: Option<bool>) -> bool {
    let a = app().autolaunch();
    // A dev build needs the dev server running, so it never registers itself;
    // Windows starts the installed copy.
    match on.filter(|_| !cfg!(debug_assertions)) {
        Some(true) => {
            let _ = a.enable();
        }
        Some(false) => {
            let _ = a.disable();
        }
        None => {}
    }
    a.is_enabled().unwrap_or(false)
}

#[derive(Serialize, Clone)]
struct Toast {
    kind: String,
    title: String,
    body: String,
    id: String,
}

fn toast(t: Toast) {
    let Some(w) = window("toast") else { return };
    let (cx, cy) = win::cursor();
    let r = win::work_area_at(cx, cy);
    let scale = w.scale_factor().unwrap_or(1.0);
    let (pw, ph) = ((380.0 * scale) as i32, (120.0 * scale) as i32);
    let _ = w.set_position(PhysicalPosition::new(r.right - pw - (12.0 * scale) as i32, r.bottom - ph - (12.0 * scale) as i32));
    *TOAST.lock().unwrap() = Some(t.clone());
    let _ = w.emit("toast", t);
    let h = raw(&w);
    win::show_quiet(h);
    win::layer(h, true);
}

static TOAST: Mutex<Option<Toast>> = Mutex::new(None);

/// The toast page asks once it has loaded, in case the event came first.
#[tauri::command]
async fn pending_toast() -> Option<Toast> {
    TOAST.lock().unwrap().clone()
}

#[tauri::command]
async fn toast_action(kind: String, action: String, id: String) {
    *TOAST.lock().unwrap() = None;
    if let Some(w) = window("toast") {
        win::hide(raw(&w));
    }
    match (kind.as_str(), action.as_str()) {
        ("call", "notes") => call_notes(&id),
        ("call", "quiet") => {
            let mut s = cfg();
            if !s.quiet_apps.contains(&id) {
                s.quiet_apps.push(id);
            }
            settings::save(&s);
            *SETTINGS.lock().unwrap() = Some(s);
        }
        ("remind", "snooze") => {
            if let Some((folder, mut n)) = store::find(&id) {
                n.remind = Some(store::now_ms() + 10 * 60_000);
                let _ = store::write(&folder, &n);
                changed();
            }
        }
        ("sorted", "open") => toggle_deck(true),
        ("remind", "open") => {
            if let Some((_, n)) = store::find(&id) {
                match n.pin {
                    Some(p) => show_sticky(&id, &p, true),
                    None => toggle_deck(true),
                }
            }
        }
        _ => {}
    }
}

/// A floating sticky for the call, already focused.
fn call_notes(app_name: &str) {
    let (cx, cy) = win::cursor();
    let r = win::work_area_at(cx, cy);
    let now = chrono::Local::now();
    let mut n = Note {
        id: store::new_id(),
        created: store::now_ms(),
        text: format!("## {app_name} call · {}\n\n", now.format("%H:%M")),
        color: "sky".into(),
        app: app_name.into(),
        ..Default::default()
    };
    n.pin = Some(Pin { mode: "float".into(), x: r.right - 380, y: r.top + 80, w: 320.0, h: 360.0, zoom: 1.0, ..Default::default() });
    if store::write("deck", &n).is_ok() {
        remember_pins();
        show_sticky(&n.id, n.pin.as_ref().unwrap(), true);
        changed();
    }
}

// ---------------------------------------------------------- background

/// Whether a scheduled Sort is due: "hourly" an hour after the last one (or
/// soon after start), "evening" once a day after 20:00.
fn sort_due(mode: &str, last: Option<chrono::DateTime<chrono::Local>>, now: chrono::DateTime<chrono::Local>) -> bool {
    use chrono::Timelike;
    match mode {
        "hourly" => last.is_none_or(|l| now - l >= chrono::Duration::hours(1)),
        "evening" => now.hour() >= 20 && last.is_none_or(|l| l.date_naive() != now.date_naive()),
        _ => false,
    }
}

/// Sorts on the schedule from Settings, in the background, then says so.
fn auto_sort(s: &Settings) {
    if s.sort_auto == "off" || SORTING.load(Ordering::Relaxed) || !ai::configured(s) {
        return;
    }
    let now = chrono::Local::now();
    if !sort_due(&s.sort_auto, *LAST_SORT.lock().unwrap(), now) {
        return;
    }
    *LAST_SORT.lock().unwrap() = Some(now);
    let notes: Vec<Note> = store::list("deck").into_iter().filter(|n| n.pin.is_none()).collect();
    if notes.is_empty() {
        return;
    }
    SORTING.store(true, Ordering::Relaxed);
    let s = s.clone();
    std::thread::spawn(move || {
        let body = match ai::plan(&s, &notes) {
            Ok(steps) => {
                let r = sort_steps(&steps);
                match (r.done, r.errors.first()) {
                    (0, Some(e)) => Some(format!("Sort failed: {e}")),
                    (0, None) => None,
                    (1, _) => Some("Sorted 1 note".to_string()),
                    (n, _) => Some(format!("Sorted {n} notes")),
                }
            }
            Err(e) => Some(format!("Sort failed: {e}")),
        };
        if let Some(body) = body {
            toast(Toast { kind: "sorted".into(), title: "Sorted".into(), body, id: String::new() });
        }
        SORTING.store(false, Ordering::Relaxed);
    });
}

fn watch() {
    std::thread::spawn(|| {
        let mut deck = store::stamp_of_deck();
        // Calls already running when Tuck starts are not news.
        let mut calls: Vec<&'static str> = calls::active_calls();
        let mut call_since: HashMap<&'static str, u32> = calls.iter().map(|c| (*c, 2)).collect();
        let mut tick = 0u32;
        loop {
            std::thread::sleep(Duration::from_secs(2));
            tick += 1;

            let now = store::stamp_of_deck();
            if now != deck {
                deck = now;
                remember_pins();
                let live: Vec<String> = pins().keys().map(|id| sticky_label(id)).collect();
                for (label, w) in app().webview_windows() {
                    if label.starts_with("sticky-") && !live.contains(&label) {
                        let _ = w.destroy();
                    }
                }
                changed();
            }

            if tick % 5 == 0 {
                let t = store::now_ms();
                for mut n in store::list("deck").into_iter().filter(|n| n.remind.is_some_and(|r| r <= t)) {
                    n.remind = None;
                    if store::write("deck", &n).is_ok() {
                        let first = n.text.lines().find(|l| !l.trim().is_empty()).unwrap_or("Reminder").trim_start_matches(['#', ' ']).to_string();
                        toast(Toast { kind: "remind".into(), title: "Reminder".into(), body: first, id: n.id.clone() });
                        changed();
                    }
                }
            }

            let s = cfg();
            if tick % 30 == 0 {
                auto_sort(&s);
            }
            if s.calls && tick % 2 == 0 {
                let active = calls::active_calls();
                for a in &active {
                    let seen = call_since.entry(a).or_insert(0);
                    *seen += 1;
                    // Four seconds of steady mic use, once per call.
                    if *seen == 2 && !calls.contains(a) && !s.quiet_apps.iter().any(|q| q == a) && !win::front_is_fullscreen() {
                        let body = if *a == "Browser" { "Call started in your browser".to_string() } else { format!("{a} call started") };
                        toast(Toast { kind: "call".into(), title: "Take notes?".into(), body, id: (*a).into() });
                    }
                }
                call_since.retain(|k, _| active.contains(k));
                calls = active.into_iter().filter(|a| call_since.get(a).copied().unwrap_or(0) >= 2).collect();
            }
        }
    });
}

fn tray(app: &AppHandle) -> tauri::Result<()> {
    let s = cfg();
    let open = MenuItem::with_id(app, "deck", "Open deck", true, None::<&str>)?;
    let new = MenuItem::with_id(app, "new", format!("New note\t{}", s.capture_key), true, None::<&str>)?;
    let sticky = MenuItem::with_id(app, "sticky", format!("New sticky\t{}", s.sticky_key), true, None::<&str>)?;
    let folder = MenuItem::with_id(app, "folder", "Show notes folder", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Tuck", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&open, &sep, &new, &sticky, &folder, &sep2, &quit])?;
    TrayIconBuilder::with_id("tuck")
        .icon(app.default_window_icon().cloned().expect("icon"))
        .tooltip("Tuck")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, e| match e.id.as_ref() {
            "deck" => toggle_deck(true),
            "new" => open_capture("note", None, ""),
            "sticky" => open_capture("sticky", None, ""),
            "folder" => open_folder("notes"),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|_, e| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                toggle_deck(false);
            }
        })
        .build(app)?;
    Ok(())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|_, _, _| toggle_deck(true)))
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, Some(vec!["--quiet"])))
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|_, sc, ev| {
                    let what = KEYS.lock().unwrap().iter().find(|(k, _)| k == sc).map(|(_, w)| *w);
                    if ev.state() != ShortcutState::Pressed {
                        return;
                    }
                    match what {
                        Some("capture") => open_capture("note", None, ""),
                        Some("sticky") => open_capture("sticky", None, ""),
                        Some("deck") => toggle_deck(false),
                        _ => {}
                    }
                })
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            ctx,
            notes,
            note,
            save,
            update,
            remove,
            restore,
            send,
            send_all,
            pin,
            sticky_geom,
            get_settings,
            set_settings,
            vaults,
            found_apps,
            anytype_challenge,
            notion_pages,
            connect,
            sort_plan,
            sort_apply,
            sort_state,
            providers,
            models,
            openrouter_login,
            cli_login,
            test_ai,
            open_url,
            dismiss,
            log,
            hide_window,
            open_deck,
            new_capture,
            reveal,
            install_skill,
            autostart,
            pending_toast,
            toast_action
        ])
        .setup(|app| {
            let _ = APP.set(app.handle().clone());
            let first_run = !std::path::Path::new(&std::env::var("APPDATA").unwrap_or_default()).join("Tuck").join("settings.json").exists();
            let s = settings::load();
            settings::save(&s);
            *SETTINGS.lock().unwrap() = Some(s.clone());
            store::set_root(&s.inbox);
            // Older builds kept deck/sent/trash folders side by side; fold them into the new layout.
            let _ = store::relocate(&store::root(), &store::root());
            let _ = store::folder("deck");
            if s.skill_auto {
                let _ = skill::install(&s);
            }
            let f = win::front();
            *FRONT.lock().unwrap() = (f.hwnd, f.exe);
            let refused = apply();
            if !refused.is_empty() {
                eprintln!("shortcuts refused: {refused:?}");
            }
            tray(app.handle())?;
            if first_run && !cfg!(debug_assertions) {
                let _ = app.autolaunch().enable();
            }
            win::start_hooks(|h| match h {
                win::Hook::Hold(x, y) => {
                    HELD.store(true, Ordering::Relaxed);
                    open_capture("note", Some((x, y)), "");
                }
                win::Hook::HoldEnd => HELD.store(false, Ordering::Relaxed),
                win::Hook::Front(h, exe) => on_front(h, exe),
                win::Hook::Moved(h) => on_moved(h),
            });
            remember_pins();
            for (id, pin) in pins() {
                show_sticky(&id, &pin, false);
            }
            watch();
            if !s.seen_intro {
                toggle_deck(true);
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("tuck failed to start");
}

#[cfg(test)]
mod tests {
    use chrono::{Local, TimeZone};

    #[test]
    fn schedule() {
        let at = |d: u32, h: u32, m: u32| Local.with_ymd_and_hms(2026, 11, d, h, m, 0).unwrap();
        // Hourly: right after start, then not before an hour has passed.
        assert!(super::sort_due("hourly", None, at(10, 9, 0)));
        assert!(!super::sort_due("hourly", Some(at(10, 9, 0)), at(10, 9, 59)));
        assert!(super::sort_due("hourly", Some(at(10, 9, 0)), at(10, 10, 0)));
        // Evening: not before 20:00, once after it, again the next evening.
        assert!(!super::sort_due("evening", None, at(10, 19, 59)));
        assert!(super::sort_due("evening", None, at(10, 20, 0)));
        assert!(!super::sort_due("evening", Some(at(10, 20, 1)), at(10, 23, 30)));
        assert!(!super::sort_due("evening", Some(at(10, 20, 1)), at(11, 8, 0)));
        assert!(super::sort_due("evening", Some(at(10, 20, 1)), at(11, 20, 0)));
        assert!(!super::sort_due("off", None, at(10, 21, 0)));
    }
}
