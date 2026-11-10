//! Note apps besides Obsidian and plain folders: finding them on this PC,
//! connecting in a click or two, and delivering a note to each.
//!
//! Local apps (Logseq, Joplin, Anytype) are found or paired without any
//! account step. Cloud apps (Notion, Todoist, Tana) take a token the person
//! copies from the app once.

use crate::dest::{file_title, http, moment};
use crate::settings::{Dest, open};
use crate::store::Note;
use chrono::{Local, TimeZone};
use serde::Serialize;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::Duration;

const ANYTYPE: &str = "http://localhost:31009/v1";
const ANYTYPE_VERSION: &str = "2025-11-08";

#[derive(Serialize)]
pub struct Found {
    pub kind: String,
    pub name: String,
    pub path: String,
}

#[derive(Serialize)]
pub struct Page {
    pub id: String,
    pub title: String,
}

fn port_open(port: u16) -> bool {
    std::net::TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_millis(200)).is_ok()
}

/// What is on this PC right now: vaults, Logseq graphs, and running local apps.
pub fn found() -> Vec<Found> {
    let mut out: Vec<Found> = crate::dest::vaults().into_iter().map(|v| Found { kind: "obsidian".into(), name: v.name, path: v.path }).collect();
    out.extend(logseq_graphs().into_iter().map(|p| Found { kind: "logseq".into(), name: name_of(&p), path: p.display().to_string() }));
    let (joplin, anytype) = std::thread::scope(|sc| {
        let a = sc.spawn(|| port_open(31009));
        (joplin_base().is_some(), a.join().unwrap_or(false))
    });
    if joplin {
        out.push(Found { kind: "joplin".into(), name: "Joplin".into(), path: String::new() });
    }
    if anytype {
        out.push(Found { kind: "anytype".into(), name: "Anytype".into(), path: String::new() });
    }
    out
}

fn name_of(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
}

/// Logseq keeps one cache file per graph, named after the graph's folder.
fn logseq_graphs() -> Vec<PathBuf> {
    let dir = PathBuf::from(std::env::var("USERPROFILE").unwrap_or_default()).join(".logseq").join("graphs");
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    rd.flatten().filter_map(|e| logseq_path(&e.file_name().to_string_lossy())).filter(|p| p.is_dir()).collect()
}

/// `logseq_local_C%3A++Users++me++notes.transit` → `C:\Users\me\notes`.
fn logseq_path(file: &str) -> Option<PathBuf> {
    let enc = file.strip_prefix("logseq_local_")?.strip_suffix(".transit")?;
    let decoded = enc.replace("%3A", ":").replace("%3a", ":").replace("%20", " ");
    let parts: Vec<&str> = decoded.split('+').filter(|s| !s.is_empty()).collect();
    if parts.is_empty() {
        return None;
    }
    let first = if parts[0].ends_with(':') { format!("{}\\", parts[0]) } else { parts[0].to_string() };
    Some(parts[1..].iter().fold(PathBuf::from(first), |p, s| p.join(s)))
}

// ------------------------------------------------------------- connecting

/// Joplin's clipper service answers on the first free port from 41184.
fn joplin_base() -> Option<String> {
    // Ports are probed at once: on Windows a closed local port can take its whole timeout.
    let open: Vec<u16> = std::thread::scope(|sc| {
        let probes: Vec<_> = (41184..=41194).map(|p| sc.spawn(move || port_open(p).then_some(p))).collect();
        probes.into_iter().filter_map(|t| t.join().ok().flatten()).collect()
    });
    open.into_iter().map(|p| format!("http://127.0.0.1:{p}")).find(|base| {
        http().get(&format!("{base}/ping")).call().ok().and_then(|mut r| r.body_mut().read_to_string().ok()).is_some_and(|t| t.contains("JoplinClipperServer"))
    })
}

/// Asks Joplin for a token; the person clicks Allow in Joplin. Waits two minutes.
pub fn joplin_connect() -> Result<Dest, String> {
    let base = joplin_base().ok_or("Open Joplin and turn on Web Clipper: Tools → Options → Web Clipper")?;
    let mut r = http().post(&format!("{base}/auth")).send_empty().map_err(|e| e.to_string())?;
    let v: Value = r.body_mut().read_json().map_err(|e| e.to_string())?;
    let auth = v["auth_token"].as_str().ok_or("Joplin didn't answer")?.to_string();
    for _ in 0..120 {
        std::thread::sleep(Duration::from_secs(1));
        let Ok(mut r) = http().get(&format!("{base}/auth/check?auth_token={auth}")).call() else { continue };
        let v: Value = r.body_mut().read_json().unwrap_or(Value::Null);
        match v["status"].as_str() {
            Some("accepted") => {
                let token = v["token"].as_str().unwrap_or_default();
                return Ok(Dest { kind: "joplin".into(), name: "Joplin".into(), path: base, token: crate::settings::seal(token), ..Default::default() });
            }
            Some("rejected") => return Err("Joplin said no".into()),
            _ => {}
        }
    }
    Err("Joplin didn't answer in time".into())
}

/// Anytype shows a four-digit code; this starts that.
pub fn anytype_challenge() -> Result<String, String> {
    let mut r = http()
        .post(&format!("{ANYTYPE}/auth/challenges"))
        .header("Anytype-Version", ANYTYPE_VERSION)
        .send_json(json!({ "app_name": "Tuck" }))
        .map_err(|_| "Open Anytype first".to_string())?;
    let v: Value = r.body_mut().read_json().map_err(|e| e.to_string())?;
    v["challenge_id"].as_str().map(String::from).ok_or("Anytype didn't answer".into())
}

pub fn anytype_connect(challenge: &str, code: &str) -> Result<Dest, String> {
    let mut r = http()
        .post(&format!("{ANYTYPE}/auth/api_keys"))
        .header("Anytype-Version", ANYTYPE_VERSION)
        .send_json(json!({ "challenge_id": challenge, "code": code.trim() }))
        .map_err(|e| e.to_string())?;
    if !r.status().is_success() {
        return Err("That code didn't match".into());
    }
    let v: Value = r.body_mut().read_json().map_err(|e| e.to_string())?;
    let key = v["api_key"].as_str().ok_or("Anytype didn't answer")?.to_string();
    let mut r = http()
        .get(&format!("{ANYTYPE}/spaces"))
        .header("Anytype-Version", ANYTYPE_VERSION)
        .header("Authorization", &format!("Bearer {key}"))
        .call()
        .map_err(|e| e.to_string())?;
    let v: Value = r.body_mut().read_json().unwrap_or(Value::Null);
    let space = &v["data"][0];
    let id = space["id"].as_str().ok_or("Anytype has no space yet")?;
    let name = space["name"].as_str().filter(|n| !n.is_empty()).unwrap_or("Anytype");
    Ok(Dest { kind: "anytype".into(), name: name.into(), page: id.into(), token: crate::settings::seal(&key), ..Default::default() })
}

/// Checks a cloud token before saving it.
pub fn token_connect(kind: &str, token: &str) -> Result<Dest, String> {
    let token = token.trim();
    if token.is_empty() {
        return Err("Paste the token first".into());
    }
    let name = match kind {
        "todoist" => {
            let r = http().get("https://api.todoist.com/api/v1/projects").header("Authorization", &format!("Bearer {token}")).call().map_err(|e| e.to_string())?;
            if !r.status().is_success() {
                return Err("Todoist didn't accept that token".into());
            }
            "Todoist"
        }
        "tana" => "Tana",
        _ => return Err("Unknown app".into()),
    };
    Ok(Dest { kind: kind.into(), name: name.into(), token: crate::settings::seal(token), ..Default::default() })
}

/// Pages the Notion integration can see, so nobody pastes page links.
pub fn notion_pages(token: &str) -> Result<Vec<Page>, String> {
    let mut r = http()
        .post("https://api.notion.com/v1/search")
        .header("Authorization", &format!("Bearer {}", token.trim()))
        .header("Notion-Version", "2022-06-28")
        .send_json(json!({ "filter": { "property": "object", "value": "page" }, "page_size": 50 }))
        .map_err(|e| format!("Notion is unreachable: {e}"))?;
    if r.status().as_u16() == 401 {
        return Err("Notion didn't accept that token".into());
    }
    let v: Value = r.body_mut().read_json().map_err(|e| e.to_string())?;
    Ok(v["results"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| {
            let title = p["properties"].as_object()?.values().find(|x| x["type"] == "title")?["title"].as_array()?.iter().filter_map(|t| t["plain_text"].as_str()).collect::<String>();
            Some(Page { id: p["id"].as_str()?.replace('-', ""), title: if title.is_empty() { "Untitled".into() } else { title } })
        })
        .collect())
}

// ------------------------------------------------------------- delivery

pub fn deliver(d: &Dest, n: &Note, keep_source: bool) -> Result<String, String> {
    match d.kind.as_str() {
        "logseq" => logseq(d, n),
        "joplin" => joplin(d, n),
        "anytype" => anytype(d, n),
        "todoist" => todoist(d, n),
        "tana" => tana(d, n),
        _ => crate::dest::deliver(d, n, keep_source),
    }
}

fn when(n: &Note) -> chrono::DateTime<Local> {
    Local.timestamp_millis_opt(n.created).single().unwrap_or_else(Local::now)
}

fn first_and_rest(text: &str) -> (String, String) {
    let t = text.trim();
    let (first, rest) = t.split_once('\n').unwrap_or((t, ""));
    (first.trim().to_string(), rest.trim().to_string())
}

fn fail(app: &str, r: &mut ureq::http::Response<ureq::Body>) -> String {
    match r.status().as_u16() {
        401 | 403 => format!("{app} no longer accepts Tuck's token. Connect it again."),
        code => format!("{app} said {code}: {}", r.body_mut().read_to_string().unwrap_or_default().chars().take(160).collect::<String>()),
    }
}

/// Today's journal page, as one block with the note under it.
fn logseq(d: &Dest, n: &Note) -> Result<String, String> {
    let root = PathBuf::from(&d.path);
    if !root.is_dir() {
        return Err(format!("{} is not a folder anymore", d.path));
    }
    let config = std::fs::read_to_string(root.join("logseq").join("config.edn")).unwrap_or_default();
    let format = edn_string(&config, ":journal/file-name-format").unwrap_or_else(|| "yyyy_MM_dd".into());
    let dir = edn_string(&config, ":journals-directory").unwrap_or_else(|| "journals".into());
    let name = when(n).format(&moment(&format.replace("yyyy", "YYYY").replace("dd", "DD"))).to_string();
    let path = root.join(dir).join(format!("{name}.md"));
    let text = n.text.trim();
    let mut block = String::new();
    for (i, line) in text.lines().enumerate() {
        block.push_str(&if i == 0 { format!("- {}\n", line.trim_start_matches("- ")) } else { format!("  {line}\n") });
    }
    crate::dest::append(&path, &block)?;
    Ok(name)
}

/// `:key "value"` from Logseq's config.edn, ignoring commented lines.
fn edn_string(config: &str, key: &str) -> Option<String> {
    config.lines().filter(|l| !l.trim_start().starts_with(';')).find_map(|l| {
        let rest = l.split_once(key)?.1.trim_start();
        let rest = rest.strip_prefix('"')?;
        Some(rest.split_once('"')?.0.to_string())
    })
}

fn joplin(d: &Dest, n: &Note) -> Result<String, String> {
    let token = open(&d.token);
    let mut body = json!({ "title": file_title(&n.text), "body": n.text.trim() });
    if !d.page.is_empty() {
        body["parent_id"] = json!(d.page);
    }
    let mut r = http().post(&format!("{}/notes?token={token}", d.path)).send_json(body).map_err(|_| "Open Joplin to send there".to_string())?;
    if r.status().is_success() { Ok(d.name.clone()) } else { Err(fail("Joplin", &mut r)) }
}

fn anytype(d: &Dest, n: &Note) -> Result<String, String> {
    let mut r = http()
        .post(&format!("{ANYTYPE}/spaces/{}/objects", d.page))
        .header("Anytype-Version", ANYTYPE_VERSION)
        .header("Authorization", &format!("Bearer {}", open(&d.token)))
        .send_json(json!({ "name": file_title(&n.text), "type_key": "page", "body": n.text.trim() }))
        .map_err(|_| "Open Anytype to send there".to_string())?;
    if r.status().is_success() { Ok(d.name.clone()) } else { Err(fail("Anytype", &mut r)) }
}

/// Todoist reads the first line its own way (dates, #project, @label); the rest becomes the description.
fn todoist(d: &Dest, n: &Note) -> Result<String, String> {
    let token = open(&d.token);
    let (first, rest) = first_and_rest(&n.text);
    let auth = format!("Bearer {token}");
    let mut r = http().post("https://api.todoist.com/api/v1/tasks/quick").header("Authorization", &auth).send_json(json!({ "text": first })).map_err(|e| e.to_string())?;
    if !r.status().is_success() {
        return Err(fail("Todoist", &mut r));
    }
    if !rest.is_empty() {
        let v: Value = r.body_mut().read_json().unwrap_or(Value::Null);
        if let Some(id) = v["id"].as_str() {
            let _ = http().post(&format!("https://api.todoist.com/api/v1/tasks/{id}")).header("Authorization", &auth).send_json(json!({ "description": rest }));
        }
    }
    Ok(d.name.clone())
}

/// Into the Tana inbox: the first line is the node, the other lines its children.
fn tana(d: &Dest, n: &Note) -> Result<String, String> {
    let (first, rest) = first_and_rest(&n.text);
    let children: Vec<Value> = rest.lines().map(str::trim).filter(|l| !l.is_empty()).map(|l| json!({ "name": l.trim_start_matches("- ") })).collect();
    let mut r = http()
        .post("https://europe-west1-tagr-prod.cloudfunctions.net/addToNodeV2")
        .header("Authorization", &format!("Bearer {}", open(&d.token)))
        .send_json(json!({ "targetNodeId": "INBOX", "nodes": [{ "name": first, "children": children }] }))
        .map_err(|e| e.to_string())?;
    if r.status().is_success() { Ok(d.name.clone()) } else { Err(fail("Tana", &mut r)) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What this PC has. `cargo test live_found -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_found() {
        for f in found() {
            println!("{} {} {}", f.kind, f.name, f.path);
        }
    }

    #[test]
    fn logseq_names_and_config() {
        assert_eq!(logseq_path("logseq_local_C%3A++Users++me++notes.transit"), Some(PathBuf::from("C:\\Users\\me\\notes")));
        assert_eq!(logseq_path("logseq_local_E%3A+++Users++MyUser++Logseq.transit"), Some(PathBuf::from("E:\\Users\\MyUser\\Logseq")));
        assert_eq!(logseq_path("other.transit"), None);
        let edn = ";; :journal/file-name-format \"x\"\n :journal/file-name-format \"yyyy-MM-dd\"\n";
        assert_eq!(edn_string(edn, ":journal/file-name-format").as_deref(), Some("yyyy-MM-dd"));
        assert_eq!(first_and_rest("  buy milk tomorrow\n- oat\n"), ("buy milk tomorrow".into(), "- oat".into()));
    }
}
