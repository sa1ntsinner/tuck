//! Notes are plain Markdown files so people and agents can read and write
//! them without Tuck. The inbox folder (Settings, default `Documents\Tuck`)
//! holds what is waiting, one readable file per note; `.tuck\sent` and
//! `.tuck\trash` inside it hold what went out or was thrown away (dot
//! folders stay out of Obsidian's file list). A file needs no front matter;
//! anything missing gets a sensible default.

use chrono::{DateTime, Local, TimeZone};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

static INBOX: RwLock<Option<PathBuf>> = RwLock::new(None);

#[derive(Serialize, Deserialize, Clone, Default, Debug, PartialEq)]
#[serde(default)]
pub struct Pin {
    /// "float" stays on top, "desk" sits under windows, "app" follows one app.
    pub mode: String,
    pub app: String,
    pub x: i32,
    pub y: i32,
    pub w: f64,
    pub h: f64,
    pub zoom: f64,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct Note {
    pub id: String,
    pub created: i64,
    pub text: String,
    pub color: String,
    pub app: String,
    pub title: String,
    pub pin: Option<Pin>,
    pub remind: Option<i64>,
    /// A destination chosen by hand or by Tidy; beats the tag rules.
    pub to: Option<String>,
    pub sent: Option<String>,
    pub sent_at: Option<i64>,
}

pub fn default_inbox() -> PathBuf {
    std::env::var("USERPROFILE").map(PathBuf::from).unwrap_or_default().join("Documents").join("Tuck")
}

/// The folder that holds waiting notes.
pub fn root() -> PathBuf {
    INBOX.read().unwrap().clone().unwrap_or_else(default_inbox)
}

pub fn set_root(path: &str) {
    let p = if path.trim().is_empty() { default_inbox() } else { PathBuf::from(path.trim()) };
    *INBOX.write().unwrap() = Some(p);
}

/// A note folder, made on first use. The inbox itself is only made when its
/// parent exists: a missing parent means the person moved or renamed it, and
/// rebuilding the old path inside their vault would leave stray folders.
pub fn folder(name: &str) -> PathBuf {
    let inbox = root();
    if !inbox.is_dir() && inbox.parent().is_some_and(Path::is_dir) {
        let _ = fs::create_dir(&inbox);
    }
    let p = if name == "deck" { inbox.clone() } else { inbox.join(".tuck").join(name) };
    if inbox.is_dir() {
        let _ = fs::create_dir_all(&p);
    }
    p
}

/// False when the inbox folder is gone (moved or renamed outside Tuck).
pub fn inbox_ok() -> bool {
    folder("deck").is_dir()
}

/// Moves every note (waiting, sent, trashed) from one inbox to another.
/// Only Tuck's own files move: anything without a Tuck id stays where it is.
pub fn relocate(from: &Path, to: &Path) -> std::io::Result<usize> {
    if !from.is_dir() {
        return Ok(0);
    }
    // Same rule as `folder`: never rebuild a missing parent of the inbox.
    if !to.is_dir() && !to.parent().is_some_and(Path::is_dir) {
        return Err(std::io::Error::new(std::io::ErrorKind::NotFound, "the folder above it doesn't exist"));
    }
    let same = from == to;
    let mut moved = 0;
    for sub in ["", ".tuck/sent", ".tuck/trash", "deck", "sent", "trash"] {
        // Moving onto itself only folds the old deck/sent/trash layout in.
        if same && !matches!(sub, "deck" | "sent" | "trash") {
            continue;
        }
        let src = if sub.is_empty() { from.to_path_buf() } else { from.join(sub) };
        let dst = match sub {
            "" | "deck" => to.to_path_buf(),
            "sent" | ".tuck/sent" => to.join(".tuck").join("sent"),
            _ => to.join(".tuck").join("trash"),
        };
        let Ok(rd) = fs::read_dir(&src) else { continue };
        fs::create_dir_all(&dst)?;
        for e in rd.flatten() {
            let path = e.path();
            if path.extension().is_none_or(|x| x != "md") {
                continue;
            }
            // In the inbox itself only files Tuck wrote move; legacy sub-folders were Tuck's alone.
            if sub.is_empty() && !fs::read_to_string(&path).unwrap_or_default().replace("\r\n", "\n").starts_with("---\nid: ") {
                continue;
            }
            let target = free(&dst, &path.file_stem().unwrap().to_string_lossy());
            if fs::rename(&path, &target).is_err() {
                fs::copy(&path, &target)?;
                fs::remove_file(&path)?;
            }
            moved += 1;
        }
        if matches!(sub, "deck" | "sent" | "trash") {
            let _ = fs::remove_dir(&src);
        }
    }
    Ok(moved)
}

fn free(dir: &Path, stem: &str) -> PathBuf {
    let mut path = dir.join(format!("{stem}.md"));
    let mut i = 2;
    while path.exists() {
        path = dir.join(format!("{stem} {i}.md"));
        i += 1;
    }
    path
}

/// A readable file name from the note's first line, as Obsidian shows it.
pub fn file_stem(n: &Note) -> String {
    let first = n.text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let words: String = first
        .trim_start_matches(['#', '-', ' ', '*', '>'])
        .trim_start_matches("[ ] ")
        .trim_start_matches("[x] ")
        .split_whitespace()
        .filter(|w| !w.starts_with('#') && !w.starts_with('@'))
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .filter(|c| !"<>:\"/\\|?*[]^#".contains(*c) && !c.is_control())
        .take(50)
        .collect();
    let words = words.trim().trim_end_matches('.').trim().to_string();
    let day = Local.timestamp_millis_opt(n.created).single().unwrap_or_else(Local::now).format("%Y-%m-%d %H.%M");
    if words.is_empty() { format!("{day} Note") } else { format!("{day} {words}") }
}

pub fn now_ms() -> i64 {
    Local::now().timestamp_millis()
}

pub fn new_id() -> String {
    let n = Local::now();
    format!("{}-{:03}", n.format("%Y%m%d-%H%M%S"), n.timestamp_subsec_millis())
}

fn stamp(ms: i64) -> String {
    Local.timestamp_millis_opt(ms).single().map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, false)).unwrap_or_default()
}

fn unstamp(s: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(s.trim()).ok().map(|t| t.timestamp_millis())
}

pub fn to_markdown(n: &Note) -> String {
    let mut fm = format!("---\nid: {}\ncreated: {}\n", n.id, stamp(n.created));
    let mut line = |k: &str, v: &str| {
        if !v.is_empty() {
            fm.push_str(&format!("{k}: {}\n", v.replace('\n', " ")));
        }
    };
    line("color", &n.color);
    line("app", &n.app);
    line("title", &n.title);
    line("pin", &n.pin.as_ref().map(|p| serde_json::to_string(p).unwrap_or_default()).unwrap_or_default());
    line("remind", &n.remind.map(stamp).unwrap_or_default());
    line("to", n.to.as_deref().unwrap_or(""));
    line("sent", n.sent.as_deref().unwrap_or(""));
    line("sent_at", &n.sent_at.map(stamp).unwrap_or_default());
    format!("{fm}---\n{}\n", n.text.trim_end())
}

pub fn from_markdown(id: &str, raw: &str, modified: i64) -> Note {
    let raw = raw.replace("\r\n", "\n");
    let mut n = Note { id: id.to_string(), created: modified, ..Default::default() };
    let body = match raw.strip_prefix("---\n").and_then(|r| r.split_once("\n---")) {
        Some((head, rest)) => {
            for l in head.lines() {
                let Some((k, v)) = l.split_once(':') else { continue };
                let v = v.trim();
                match k.trim() {
                    "id" if !v.is_empty() => n.id = v.into(),
                    "created" => n.created = unstamp(v).unwrap_or(n.created),
                    "color" => n.color = v.into(),
                    "app" => n.app = v.into(),
                    "title" => n.title = v.into(),
                    "pin" => n.pin = serde_json::from_str(v).ok(),
                    "remind" => n.remind = unstamp(v),
                    "to" => n.to = Some(v.into()),
                    "sent" => n.sent = Some(v.into()),
                    "sent_at" => n.sent_at = unstamp(v),
                    _ => {}
                }
            }
            rest.strip_prefix('\n').unwrap_or(rest).to_string()
        }
        None => raw,
    };
    n.text = body.trim_matches('\n').to_string();
    n
}

pub fn list(name: &str) -> Vec<Note> {
    let mut out: Vec<Note> = fs::read_dir(folder(name))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            if path.extension()? != "md" {
                return None;
            }
            let id = path.file_stem()?.to_string_lossy().to_string();
            let modified = e.metadata().ok()?.modified().ok().map(|t| DateTime::<Local>::from(t).timestamp_millis()).unwrap_or(0);
            let raw = fs::read_to_string(&path).ok()?;
            Some(from_markdown(&id, &raw, modified))
        })
        .collect();
    out.sort_by(|a, b| b.created.cmp(&a.created));
    out
}

pub fn find(id: &str) -> Option<(String, Note)> {
    for name in ["deck", "sent", "trash"] {
        if let Some(n) = list(name).into_iter().find(|n| n.id == id) {
            return Some((name.to_string(), n));
        }
    }
    None
}

/// The file that holds a note: found by its id, which lives in the front
/// matter (or is the file name for notes written by hand).
pub fn path_of(name: &str, id: &str) -> Option<PathBuf> {
    let dir = folder(name);
    let direct = dir.join(format!("{id}.md"));
    if direct.is_file() {
        return Some(direct);
    }
    fs::read_dir(&dir).ok()?.flatten().map(|e| e.path()).find(|p| {
        p.extension().is_some_and(|x| x == "md")
            && fs::read_to_string(p).is_ok_and(|raw| {
                let raw = raw.replace("\r\n", "\n");
                raw.strip_prefix("---\n").and_then(|r| r.split_once("\n---")).is_some_and(|(head, _)| head.lines().any(|l| l.trim() == format!("id: {id}")))
            })
    })
}

pub fn write(name: &str, n: &Note) -> std::io::Result<()> {
    let path = path_of(name, &n.id).unwrap_or_else(|| free(&folder(name), &file_stem(n)));
    let tmp = path.with_extension("md.tmp");
    fs::write(&tmp, to_markdown(n))?;
    fs::rename(tmp, path)
}

pub fn shift(n: &Note, from: &str, to: &str) -> std::io::Result<()> {
    let old = path_of(from, &n.id);
    write(to, n)?;
    if let Some(p) = old {
        let _ = fs::remove_file(p);
    }
    Ok(())
}

pub fn delete(name: &str, id: &str) {
    if let Some(p) = path_of(name, id) {
        let _ = fs::remove_file(p);
    }
}

/// Changes inside the deck folder, cheap enough to poll.
pub fn stamp_of_deck() -> u64 {
    fs::read_dir(folder("deck"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.metadata().ok()?.modified().ok())
        .map(|t| t.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0))
        .fold(0u64, |acc, t| acc.wrapping_add(t))
}

pub fn tags(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for w in text.split(|c: char| c.is_whitespace() || c == ',' || c == '(' || c == ')') {
        if let Some(t) = w.strip_prefix('#') {
            let t: String = t.chars().take_while(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '/')).collect();
            if !t.is_empty() && !t.chars().all(|c| c.is_ascii_digit()) && !out.contains(&t.to_lowercase()) {
                out.push(t.to_lowercase());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_bare_files() {
        let n = Note {
            id: "x".into(),
            created: 1_700_000_000_000,
            text: "hello #Work\n- [ ] thing".into(),
            color: "butter".into(),
            app: "Code".into(),
            pin: Some(Pin { mode: "float".into(), x: 10, y: 20, w: 240.0, h: 200.0, zoom: 1.0, ..Default::default() }),
            remind: Some(1_700_000_600_000),
            ..Default::default()
        };
        let back = from_markdown("x", &to_markdown(&n), 0);
        assert_eq!(back.text, n.text);
        assert_eq!(back.created, n.created);
        assert_eq!(back.pin, n.pin);
        assert_eq!(back.remind, n.remind);
        let bare = from_markdown("y", "just a thought\r\n", 5);
        assert_eq!((bare.text.as_str(), bare.created), ("just a thought", 5));
        assert_eq!(tags("ship it #Work, #42 (#a-b) #work"), vec!["work", "a-b"]);
    }

    #[test]
    fn missing_inbox_parent_is_never_rebuilt() {
        let tmp = std::env::temp_dir().join(format!("tuck-test-store-{}", std::process::id()));
        let from = tmp.join("old");
        fs::create_dir_all(&from).unwrap();
        fs::write(from.join("a.md"), "---\nid: a\n---\nhi\n").unwrap();
        let gone = tmp.join("renamed").join("Inbox");
        assert!(relocate(&from, &gone).is_err());
        assert!(!tmp.join("renamed").exists());
        assert!(from.join("a.md").exists());
        // An inbox whose parent exists is made and gets the note.
        let ok = tmp.join("Inbox");
        assert_eq!(relocate(&from, &ok).unwrap(), 1);
        assert!(ok.join("a.md").exists());
        let _ = fs::remove_dir_all(&tmp);
    }
}
