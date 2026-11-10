//! Where notes go: an Obsidian vault, any Markdown folder, or a Notion page.

use crate::settings::{Dest, Settings, open};
use crate::store::{Note, tags};
use chrono::{Local, TimeZone};
use serde::Serialize;
use serde_json::{Value, json};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Serialize)]
pub struct Vault {
    pub name: String,
    pub path: String,
}

/// The vaults Obsidian knows about, newest first.
pub fn vaults() -> Vec<Vault> {
    let file = PathBuf::from(std::env::var("APPDATA").unwrap_or_default()).join("obsidian").join("obsidian.json");
    let Some(json) = fs::read_to_string(file).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok()) else { return Vec::new() };
    let mut list: Vec<(i64, Vault)> = json["vaults"]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(_, v)| {
            let path = v["path"].as_str()?.to_string();
            let name = Path::new(&path).file_name()?.to_string_lossy().to_string();
            Path::new(&path).exists().then(|| (v["ts"].as_i64().unwrap_or(0), Vault { name, path }))
        })
        .collect();
    list.sort_by(|a, b| b.0.cmp(&a.0));
    list.into_iter().map(|(_, v)| v).collect()
}

/// A tag rule wins, then the default destination.
pub fn route<'a>(s: &'a Settings, n: &Note) -> Option<&'a Dest> {
    let note_tags = tags(&n.text);
    s.rules
        .iter()
        .find(|r| note_tags.iter().any(|t| t.eq_ignore_ascii_case(r.tag.trim_start_matches('#'))))
        .and_then(|r| s.dests.iter().find(|d| d.id == r.dest))
        .or_else(|| s.dests.iter().find(|d| d.id == s.default_dest))
}

pub fn deliver(d: &Dest, n: &Note, keep_source: bool) -> Result<String, String> {
    match d.kind.as_str() {
        "notion" => notion(d, n, keep_source),
        _ => markdown(d, n, keep_source),
    }
}

fn when(n: &Note) -> chrono::DateTime<Local> {
    Local.timestamp_millis_opt(n.created).single().unwrap_or_else(Local::now)
}

fn source(n: &Note, keep: bool) -> String {
    if keep && !n.app.is_empty() { format!(" — *{}*", n.app) } else { String::new() }
}

/// One entry for a daily note or an inbox file.
pub fn entry(n: &Note, keep_source: bool) -> String {
    let time = when(n).format("%H:%M");
    let text = n.text.trim();
    let single = !text.contains('\n') && !text.starts_with("- ") && !text.starts_with("#");
    if single {
        return format!("- {time} {text}{}\n", source(n, keep_source));
    }
    let body: String = text.lines().map(|l| format!("  {l}\n")).collect();
    format!("- {time}{}\n{body}", source(n, keep_source))
}

fn markdown(d: &Dest, n: &Note, keep_source: bool) -> Result<String, String> {
    let root = PathBuf::from(&d.path);
    if !root.is_dir() {
        return Err(format!("{} is not a folder anymore", d.path));
    }
    match d.mode.as_str() {
        "note" => {
            let dir = root.join(if d.file.trim().is_empty() { "Inbox" } else { d.file.trim() });
            fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            let path = free_name(&dir, &file_title(&n.text));
            let mut out = format!("---\ncreated: {}\n", when(n).format("%Y-%m-%dT%H:%M"));
            let t = tags(&n.text);
            if !t.is_empty() {
                out.push_str(&format!("tags: [{}]\n", t.join(", ")));
            }
            if keep_source && !n.app.is_empty() {
                out.push_str(&format!("source: \"{}\"\n", n.app.replace('"', "'")));
            }
            out.push_str(&format!("---\n\n{}\n", n.text.trim()));
            fs::write(&path, out).map_err(|e| e.to_string())?;
            Ok(path.file_name().unwrap_or_default().to_string_lossy().into())
        }
        "inbox" => {
            let name = if d.file.trim().is_empty() { "Inbox.md" } else { d.file.trim() };
            let path = root.join(if name.ends_with(".md") { name.to_string() } else { format!("{name}.md") });
            append(&path, &entry(n, keep_source))?;
            Ok(name.into())
        }
        _ => {
            let path = daily_path(d, &root, when(n));
            append(&path, &entry(n, keep_source))?;
            Ok(path.file_stem().unwrap_or_default().to_string_lossy().into())
        }
    }
}

pub fn append(path: &Path, text: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let existing = fs::read_to_string(path).unwrap_or_default();
    let lead = if existing.is_empty() || existing.ends_with('\n') { "" } else { "\n" };
    let mut f = fs::OpenOptions::new().create(true).append(true).open(path).map_err(|e| e.to_string())?;
    f.write_all(format!("{lead}{text}").as_bytes()).map_err(|e| e.to_string())
}

/// Obsidian's own daily-note settings when the folder is a vault.
fn daily_path(d: &Dest, root: &Path, day: chrono::DateTime<Local>) -> PathBuf {
    let conf: Value = fs::read_to_string(root.join(".obsidian").join("daily-notes.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Value::Null);
    let folder = conf["folder"].as_str().filter(|_| d.kind == "obsidian").unwrap_or(d.file.as_str()).to_string();
    let format = conf["format"].as_str().filter(|f| !f.is_empty() && d.kind == "obsidian").unwrap_or("YYYY-MM-DD");
    let name = day.format(&moment(format)).to_string();
    root.join(folder.trim_matches('/')).join(format!("{name}.md"))
}

/// Moment.js date tokens (what Obsidian uses) to chrono's.
pub fn moment(f: &str) -> String {
    let tokens = [
        ("YYYY", "%Y"),
        ("YY", "%y"),
        ("MMMM", "%B"),
        ("MMM", "%b"),
        ("MM", "%m"),
        ("M", "%-m"),
        ("DDDD", "%j"),
        ("DD", "%d"),
        ("Do", "%-d"),
        ("D", "%-d"),
        ("dddd", "%A"),
        ("ddd", "%a"),
        ("WW", "%V"),
        ("ww", "%U"),
        ("HH", "%H"),
        ("mm", "%M"),
    ];
    let mut out = String::new();
    let mut rest = f;
    'outer: while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix('[') {
            let (lit, after) = r.split_once(']').unwrap_or((r, ""));
            out.push_str(&lit.replace('%', "%%"));
            rest = after;
            continue;
        }
        for (m, c) in tokens {
            if let Some(r) = rest.strip_prefix(m) {
                out.push_str(c);
                rest = r;
                continue 'outer;
            }
        }
        let ch = rest.chars().next().unwrap();
        if ch == '%' {
            out.push('%');
        }
        out.push(ch);
        rest = &rest[ch.len_utf8()..];
    }
    out
}

pub fn file_title(text: &str) -> String {
    let first = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("Note");
    let cleaned: String = first
        .trim_start_matches(['#', '-', ' ', '*'])
        .trim_start_matches("[ ] ")
        .split_whitespace()
        .filter(|w| !w.starts_with('#') && !w.starts_with('@'))
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .filter(|c| !"<>:\"/\\|?*[]^".contains(*c) && !c.is_control())
        .take(60)
        .collect();
    let cleaned = cleaned.trim().trim_end_matches('.').to_string();
    if cleaned.is_empty() { "Note".into() } else { cleaned }
}

fn free_name(dir: &Path, title: &str) -> PathBuf {
    let mut path = dir.join(format!("{title}.md"));
    let mut i = 2;
    while path.exists() {
        path = dir.join(format!("{title} {i}.md"));
        i += 1;
    }
    path
}

pub fn http() -> ureq::Agent {
    ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(Duration::from_secs(45))).build().into()
}

/// The 32-hex page id from a Notion link or a bare id.
pub fn notion_id(s: &str) -> Option<String> {
    let hex: String = s.split(['?', '#']).next()?.chars().filter(|c| c.is_ascii_hexdigit() || *c == '-').filter(|c| *c != '-').collect();
    (hex.len() >= 32).then(|| hex[hex.len() - 32..].to_string())
}

fn notion(d: &Dest, n: &Note, keep_source: bool) -> Result<String, String> {
    let token = open(&d.token);
    if token.is_empty() {
        return Err("Notion needs an integration token".into());
    }
    let page = notion_id(&d.page).ok_or("That doesn't look like a Notion page link")?;
    let rich = |s: &str, bold: bool| json!([{ "type": "text", "text": { "content": s }, "annotations": { "bold": bold } }]);
    let mut blocks = vec![json!({
        "type": "paragraph",
        "paragraph": { "rich_text": rich(&format!("{}{}", when(n).format("%d %b, %H:%M"), if keep_source && !n.app.is_empty() { format!(" · {}", n.app) } else { String::new() }), true) }
    })];
    for line in n.text.lines().filter(|l| !l.trim().is_empty()) {
        let l = line.trim_start();
        let block = if let Some(t) = l.strip_prefix("- [ ] ") {
            json!({ "type": "to_do", "to_do": { "rich_text": rich(t, false), "checked": false } })
        } else if let Some(t) = l.strip_prefix("- [x] ") {
            json!({ "type": "to_do", "to_do": { "rich_text": rich(t, false), "checked": true } })
        } else if let Some(t) = l.strip_prefix("- ") {
            json!({ "type": "bulleted_list_item", "bulleted_list_item": { "rich_text": rich(t, false) } })
        } else {
            json!({ "type": "paragraph", "paragraph": { "rich_text": rich(l, false) } })
        };
        blocks.push(block);
    }
    let mut res = http()
        .patch(&format!("https://api.notion.com/v1/blocks/{page}/children"))
        .header("Authorization", &format!("Bearer {token}"))
        .header("Notion-Version", "2022-06-28")
        .send_json(json!({ "children": blocks }))
        .map_err(|e| format!("Notion is unreachable: {e}"))?;
    if res.status().is_success() {
        return Ok(d.name.clone());
    }
    let body: Value = res.body_mut().read_json().unwrap_or(Value::Null);
    Err(match res.status().as_u16() {
        401 => "Notion rejected the token".into(),
        404 => "Notion can't see that page. Share it with your integration (••• → Connections).".into(),
        code => format!("Notion said {code}: {}", body["message"].as_str().unwrap_or("")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moment_tokens() {
        assert_eq!(moment("YYYY-MM-DD"), "%Y-%m-%d");
        assert_eq!(moment("[Week] WW, dddd D MMMM"), "Week %V, %A %-d %B");
        assert_eq!(moment("YYYY/MM/YYYY-MM-DD"), "%Y/%m/%Y-%m-%d");
    }

    #[test]
    fn titles_and_ids() {
        assert_eq!(file_title("\n# Fix the PLC: timer #work @15:00\nmore"), "Fix the PLC timer");
        assert_eq!(file_title("   "), "Note");
        assert_eq!(
            notion_id("https://www.notion.so/team/Inbox-1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d?pvs=4").as_deref(),
            Some("1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d")
        );
    }

    #[test]
    fn entries() {
        let n = Note { created: 0, text: "call Anna".into(), app: "Teams".into(), ..Default::default() };
        assert!(entry(&n, true).ends_with("call Anna — *Teams*\n"));
        let n = Note { text: "- [ ] a\n- [ ] b".into(), ..Default::default() };
        assert!(entry(&n, false).contains("\n  - [ ] a\n  - [ ] b\n"));
    }
}
