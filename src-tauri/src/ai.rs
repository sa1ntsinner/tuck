//! Sort: a model reads the waiting notes plus a map of the library (its
//! folders, its key files, and its own rules for agents) and decides, note
//! by note, where each belongs. Tuck shows the plan, then writes it. The
//! model never touches files: Tuck checks every path and only appends or
//! creates, never overwrites or deletes.
//!
//! Any provider works: an API key, a local server, OpenRouter's sign-in, or
//! an agent CLI the person is already signed in to (Claude Code, Codex,
//! Gemini CLI), which uses their subscription and needs no key at all.

use crate::dest::{http, moment};
use crate::settings::{Settings, open};
use crate::store::Note;
use chrono::{Local, TimeZone};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub struct Provider {
    pub id: &'static str,
    pub name: &'static str,
    /// "cli" (signed-in agent), "key" (API key), "local" (server on this PC), "custom".
    pub kind: &'static str,
    pub base: &'static str,
}

pub const PROVIDERS: &[Provider] = &[
    Provider { id: "claude-code", name: "Claude Code", kind: "cli", base: "claude" },
    Provider { id: "codex", name: "Codex", kind: "cli", base: "codex" },
    Provider { id: "gemini-cli", name: "Gemini CLI", kind: "cli", base: "gemini" },
    Provider { id: "anthropic", name: "Anthropic", kind: "key", base: "https://api.anthropic.com/v1" },
    Provider { id: "openai", name: "OpenAI", kind: "key", base: "https://api.openai.com/v1" },
    Provider { id: "gemini", name: "Google Gemini", kind: "key", base: "https://generativelanguage.googleapis.com/v1beta/openai" },
    Provider { id: "openrouter", name: "OpenRouter", kind: "key", base: "https://openrouter.ai/api/v1" },
    Provider { id: "mistral", name: "Mistral", kind: "key", base: "https://api.mistral.ai/v1" },
    Provider { id: "groq", name: "Groq", kind: "key", base: "https://api.groq.com/openai/v1" },
    Provider { id: "deepseek", name: "DeepSeek", kind: "key", base: "https://api.deepseek.com/v1" },
    Provider { id: "xai", name: "xAI", kind: "key", base: "https://api.x.ai/v1" },
    Provider { id: "ollama", name: "Ollama", kind: "local", base: "http://localhost:11434/v1" },
    Provider { id: "lmstudio", name: "LM Studio", kind: "local", base: "http://localhost:1234/v1" },
    Provider { id: "custom", name: "Other server", kind: "custom", base: "" },
];

pub fn provider(id: &str) -> Option<&'static Provider> {
    PROVIDERS.iter().find(|p| p.id == id)
}

#[derive(Serialize)]
pub struct Status {
    pub id: String,
    pub name: String,
    pub kind: String,
    /// Usable right now without anything else from the person.
    pub ready: bool,
    pub state: String,
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var("USERPROFILE").unwrap_or_default())
}

/// The CLI's full path. An .exe wins over an npm .cmd shim.
pub fn which(name: &str) -> Option<PathBuf> {
    let out = hidden(Command::new("where.exe").arg(name)).output().ok()?;
    let found: Vec<PathBuf> = String::from_utf8_lossy(&out.stdout).lines().map(|l| PathBuf::from(l.trim())).filter(|p| p.extension().is_some()).collect();
    found.iter().find(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe"))).or_else(|| found.first()).cloned()
}

fn signed_in(id: &str) -> bool {
    let h = home();
    match id {
        "claude-code" => h.join(".claude").join(".credentials.json").exists() || std::env::var("ANTHROPIC_API_KEY").is_ok(),
        "codex" => h.join(".codex").join("auth.json").exists(),
        "gemini-cli" => h.join(".gemini").join("oauth_creds.json").exists() || std::env::var("GEMINI_API_KEY").is_ok(),
        _ => false,
    }
}

fn listening(base: &str) -> bool {
    let port = base.rsplit(':').next().and_then(|p| p.split('/').next()).and_then(|p| p.parse::<u16>().ok()).unwrap_or(80);
    std::net::TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_millis(250)).is_ok()
}

pub fn statuses() -> Vec<Status> {
    PROVIDERS
        .iter()
        .map(|p| {
            let (ready, state) = match p.kind {
                "cli" => match (which(p.base).is_some(), signed_in(p.id)) {
                    (false, _) => (false, "Not installed"),
                    (true, true) => (true, "Signed in"),
                    (true, false) => (false, "Sign in"),
                },
                "local" if listening(p.base) => (true, "Running"),
                "local" => (false, "Not running"),
                _ => (false, ""),
            };
            Status { id: p.id.into(), name: p.name.into(), kind: p.kind.into(), ready, state: state.into() }
        })
        .collect()
}

/// True when Sort can run with these settings.
pub fn configured(s: &Settings) -> bool {
    match provider(&s.ai_provider) {
        Some(p) if p.kind == "cli" || p.kind == "local" => true,
        Some(p) if p.kind == "custom" => !s.ai_url.trim().is_empty(),
        Some(_) => !s.ai_key.is_empty(),
        None => false,
    }
}

fn base_of(s: &Settings, p: &Provider) -> String {
    let custom = s.ai_url.trim().trim_end_matches('/');
    if !custom.is_empty() && p.kind != "cli" { custom.to_string() } else { p.base.to_string() }
}

/// Models the provider offers, so nobody types a model id from memory.
pub fn models(id: &str, url: &str, key: &str) -> Result<Vec<String>, String> {
    let p = provider(id).ok_or("Unknown provider")?;
    match p.id {
        "claude-code" => return Ok(vec!["opus".into(), "sonnet".into(), "haiku".into()]),
        "codex" | "gemini-cli" => return Ok(Vec::new()),
        _ => {}
    }
    let base = if url.trim().is_empty() { p.base.to_string() } else { url.trim().trim_end_matches('/').to_string() };
    if base.is_empty() {
        return Err("Add the server address".into());
    }
    let mut req = http().get(&format!("{base}/models"));
    if p.id == "anthropic" {
        req = req.header("x-api-key", key).header("anthropic-version", "2026-10-21");
    } else if !key.is_empty() {
        req = req.header("Authorization", &format!("Bearer {key}"));
    }
    let mut res = req.call().map_err(|e| format!("Can't reach {base}: {e}"))?;
    let status = res.status().as_u16();
    let v: Value = res.body_mut().read_json().unwrap_or(Value::Null);
    match status {
        200 => {}
        401 | 403 => return Err("The key was rejected".into()),
        _ => return Err(format!("The server said {status}")),
    }
    let mut list: Vec<String> = v["data"]
        .as_array()
        .or(v["models"].as_array())
        .into_iter()
        .flatten()
        .filter_map(|m| m["id"].as_str().or(m["name"].as_str()))
        .map(|m| m.trim_start_matches("models/").to_string())
        .filter(|m| !["embed", "tts", "whisper", "audio", "image", "dall-e", "moderation", "realtime", "transcribe"].iter().any(|x| m.contains(x)))
        .collect();
    list.dedup();
    Ok(list)
}

// ------------------------------------------------------------------ ask

/// One request, one answer, from whatever the person chose.
pub fn ask(s: &Settings, system: &str, user: &str, schema: &Value) -> Result<String, String> {
    let p = provider(&s.ai_provider).ok_or("Choose who sorts your notes in Settings")?;
    let key = open(&s.ai_key);
    match p.kind {
        "cli" => cli(p, s.ai_model.trim(), &format!("{system}\n\nAnswer with JSON only, shaped like this schema:\n{schema}\n\n{user}")),
        _ if p.id == "anthropic" => anthropic(s, p, &key, system, user, schema),
        _ => openai(s, p, &key, system, user, schema),
    }
}

fn anthropic(s: &Settings, p: &Provider, key: &str, system: &str, user: &str, schema: &Value) -> Result<String, String> {
    if key.is_empty() {
        return Err("Add an Anthropic API key in Settings".into());
    }
    let model = if s.ai_model.trim().is_empty() { "claude-opus-5-5" } else { s.ai_model.trim() };
    let mut body = json!({
        "model": model,
        "max_tokens": 16000,
        "system": system,
        "output_config": { "effort": "low", "format": { "type": "json_schema", "schema": schema } },
        "messages": [{ "role": "user", "content": user }]
    });
    let fallback = model.starts_with("claude-opus-5") || model.starts_with("claude-fable") || model.starts_with("claude-sonnet-5-5");
    if fallback {
        body["fallbacks"] = json!("default");
    }
    // The server address can be changed in Settings, e.g. for a proxy.
    let mut req = http().post(&format!("{}/messages", base_of(s, p))).header("x-api-key", key).header("anthropic-version", "2026-10-21");
    if fallback {
        req = req.header("anthropic-beta", "server-side-fallback-2026-07-01");
    }
    let mut res = req.send_json(body).map_err(|e| format!("Can't reach Anthropic: {e}"))?;
    let status = res.status().as_u16();
    let v: Value = res.body_mut().read_json().unwrap_or(Value::Null);
    match status {
        200 => {}
        401 => return Err("Anthropic rejected the API key".into()),
        429 => return Err("Anthropic is rate limiting this key. Try again in a minute.".into()),
        _ => return Err(format!("Anthropic said {status}: {}", v["error"]["message"].as_str().unwrap_or(""))),
    }
    if v["stop_reason"] == "refusal" {
        return Err("The model declined to sort these notes".into());
    }
    v["content"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|b| b["type"] == "text")
        .and_then(|b| b["text"].as_str())
        .map(String::from)
        .ok_or_else(|| "The model returned no answer".into())
}

/// OpenAI's Chat Completions shape, which every other API here speaks too.
fn openai(s: &Settings, p: &Provider, key: &str, system: &str, user: &str, schema: &Value) -> Result<String, String> {
    let base = base_of(s, p);
    if base.is_empty() {
        return Err("Add the server address in Settings".into());
    }
    if p.kind == "key" && key.is_empty() {
        return Err(format!("Add a {} API key in Settings", p.name));
    }
    let model = s.ai_model.trim();
    if model.is_empty() {
        return Err("Pick a model in Settings".into());
    }
    let mut body = json!({
        "model": model,
        "messages": [
            { "role": "system", "content": format!("{system}\n\nAnswer with JSON only, shaped like this schema:\n{schema}") },
            { "role": "user", "content": user }
        ]
    });
    if !matches!(p.id, "lmstudio" | "custom") {
        body["response_format"] = json!({ "type": "json_object" });
    }
    let mut req = http().post(&format!("{base}/chat/completions"));
    if !key.is_empty() {
        req = req.header("Authorization", &format!("Bearer {key}"));
    }
    if p.id == "openrouter" {
        req = req.header("X-Title", "Tuck");
    }
    let mut res = req.send_json(body).map_err(|e| format!("Can't reach {}: {e}", p.name))?;
    let status = res.status().as_u16();
    let v: Value = res.body_mut().read_json().unwrap_or(Value::Null);
    match status {
        200 => {}
        401 | 403 => return Err(format!("{} rejected the key", p.name)),
        429 => return Err(format!("{} is rate limiting. Try again in a minute.", p.name)),
        _ => return Err(format!("{} said {status}: {}", p.name, v["error"]["message"].as_str().or(v["error"].as_str()).unwrap_or(""))),
    }
    v["choices"][0]["message"]["content"].as_str().map(String::from).ok_or_else(|| "The model returned no answer".into())
}

pub fn hidden(cmd: &mut Command) -> &mut Command {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(0x0800_0000) // CREATE_NO_WINDOW
}

/// Runs a signed-in agent CLI once, without tools, and returns its answer.
fn cli(p: &Provider, model: &str, prompt: &str) -> Result<String, String> {
    let exe = which(p.base).ok_or(format!("{} is not installed", p.name))?;
    let dir = std::env::temp_dir().join("tuck-sort");
    let _ = std::fs::create_dir_all(&dir);
    let last = dir.join("answer.txt");
    let _ = std::fs::remove_file(&last);
    let mut cmd = Command::new(&exe);
    match p.id {
        "claude-code" => {
            cmd.args(["-p", "--output-format", "json", "--tools", "", "--strict-mcp-config"]);
            if !model.is_empty() {
                cmd.args(["--model", model]);
            }
        }
        "codex" => {
            cmd.args(["exec", "--skip-git-repo-check", "--ephemeral", "--sandbox", "read-only", "-o"]).arg(&last);
            if !model.is_empty() {
                cmd.args(["-m", model]);
            }
            cmd.arg("-");
        }
        _ => {
            cmd.args(["-o", "json", "-p", "Do what the input asks."]);
            if !model.is_empty() {
                cmd.args(["-m", model]);
            }
        }
    }
    let mut child = hidden(&mut cmd)
        .current_dir(&dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Can't start {}: {e}", p.name))?;
    let mut stdin = child.stdin.take().unwrap();
    let input = prompt.to_string();
    std::thread::spawn(move || {
        let _ = stdin.write_all(input.as_bytes());
    });
    let mut out = child.stdout.take().unwrap();
    let mut err = child.stderr.take().unwrap();
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = out.read_to_string(&mut s);
        s
    });
    let err_reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = err.read_to_string(&mut s);
        s
    });
    let start = Instant::now();
    let status = loop {
        if let Some(st) = child.try_wait().map_err(|e| e.to_string())? {
            break st;
        }
        if start.elapsed() > Duration::from_secs(300) {
            let _ = child.kill();
            return Err(format!("{} took longer than five minutes", p.name));
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    let stdout = reader.join().unwrap_or_default();
    let stderr = err_reader.join().unwrap_or_default();
    if !status.success() {
        let line = stderr.lines().chain(stdout.lines()).map(str::trim).filter(|l| !l.is_empty()).last().unwrap_or("it stopped with an error");
        return Err(format!("{}: {line}", p.name));
    }
    match p.id {
        "claude-code" => {
            let v: Value = serde_json::from_str(stdout.trim()).map_err(|_| "Claude Code answered in a shape Tuck can't read")?;
            if v["is_error"] == true {
                return Err(format!("Claude Code: {}", v["result"].as_str().unwrap_or("error")));
            }
            v["result"].as_str().map(String::from).ok_or_else(|| "Claude Code returned no answer".into())
        }
        "codex" => std::fs::read_to_string(&last).map_err(|_| "Codex returned no answer".into()),
        _ => Ok(serde_json::from_str::<Value>(stdout.trim()).ok().and_then(|v| v["response"].as_str().map(String::from)).unwrap_or(stdout)),
    }
}

/// The JSON object inside an answer that may carry prose or code fences.
fn json_in(raw: &str) -> Option<Value> {
    let (a, b) = (raw.find('{')?, raw.rfind('}')?);
    serde_json::from_str(raw.get(a..=b)?).ok()
}

pub fn test(s: &Settings) -> Result<(), String> {
    let schema = json!({ "type": "object", "properties": { "ok": { "type": "boolean" } }, "required": ["ok"], "additionalProperties": false });
    let raw = ask(s, "You check that a connection works.", "Answer {\"ok\": true}.", &schema)?;
    json_in(&raw).filter(|v| v["ok"] == true).map(|_| ()).ok_or_else(|| "It answered, but not in a shape Tuck can read".into())
}

// ----------------------------------------------------------------- plan

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Step {
    pub id: String,
    /// "append", "create" or "keep".
    pub action: String,
    /// Relative to the library root.
    #[serde(default)]
    pub path: String,
    /// A heading in the target file to add below; empty adds at the end.
    #[serde(default)]
    pub under: String,
    /// Exactly what gets written.
    #[serde(default)]
    pub text: String,
    /// A few words naming the place, for the preview.
    #[serde(default)]
    pub why: String,
}

const SYSTEM: &str = "You file a person's quick notes into their notes library. You get the \
library's own rules (when it has them), a map of its folders and files, today's date and \
daily note, and the notes. For each note choose one action:\n\
- append: add to an existing Markdown file from the map (or today's daily note). Set `under` to a \
heading that exists in that file when the library's rules say where things go (for example \
\"## Activity\"); otherwise leave it empty to add at the end.\n\
- create: a new Markdown file in an existing folder from the map, named the way that folder names \
its notes.\n\
- keep: leave the note waiting, when it is unclear, unfinished, or nothing fits.\n\
`text` is exactly what gets written, shaped the way the place expects: task syntax for task lists \
(with dates in the library's format when the note gives a time), a line in the daily note's style, \
a whole note with front matter when the folder's notes use it. Keep the person's words and \
language; never translate, summarise, or add facts. The library's rules beat your own taste. \
Tasks go to task lists, project facts to that project, thoughts and logs to the daily note. `why` \
names the place in at most five plain words. Use paths exactly as the map writes them.";

fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "items": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string" },
                        "action": { "type": "string", "enum": ["append", "create", "keep"] },
                        "path": { "type": "string" },
                        "under": { "type": "string" },
                        "text": { "type": "string" },
                        "why": { "type": "string" }
                    },
                    "required": ["id", "action", "path", "under", "text", "why"],
                    "additionalProperties": false
                }
            }
        },
        "required": ["items"],
        "additionalProperties": false
    })
}

/// The library folder: chosen in Settings, else the vault the inbox lives in.
pub fn library(s: &Settings) -> Option<PathBuf> {
    if !s.library.trim().is_empty() {
        return Some(PathBuf::from(s.library.trim())).filter(|p| p.is_dir());
    }
    let inbox = crate::store::root();
    inbox.ancestors().find(|a| a.join(".obsidian").is_dir()).map(Path::to_path_buf)
}

const SKIP: &[&str] = &["attachments", "assets", "node_modules", "templates", "archive"];

/// Folders and notes, compact: big folders show only their newest files.
pub fn map(lib: &Path, inbox: &Path) -> String {
    fn walk(dir: &Path, rel: &str, depth: usize, inbox: &Path, out: &mut Vec<String>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        let mut dirs = Vec::new();
        let mut files = Vec::new();
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || name.contains(".sync-conflict-") {
                continue;
            }
            let path = e.path();
            if path.is_dir() {
                if path != inbox {
                    dirs.push(name);
                }
            } else if name.ends_with(".md") {
                let t = e.metadata().and_then(|m| m.modified()).ok();
                files.push((t, name));
            }
        }
        dirs.sort();
        files.sort_by(|a, b| b.0.cmp(&a.0));
        let shown = if files.len() > 12 { 8 } else { files.len() };
        for (_, f) in files.iter().take(shown) {
            out.push(format!("{rel}{f}"));
        }
        if files.len() > shown {
            out.push(format!("{rel}… {} more notes", files.len() - shown));
        }
        for d in dirs {
            let lower = d.to_lowercase();
            out.push(format!("{rel}{d}/"));
            if depth < 4 && !SKIP.iter().any(|s| lower.contains(s)) {
                walk(&dir.join(&d), &format!("{rel}{d}/"), depth + 1, inbox, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(lib, "", 0, inbox, &mut out);
    out.truncate(700);
    out.join("\n")
}

/// The library's own instructions for agents, if it has any.
pub fn rules(lib: &Path) -> String {
    for f in ["AGENTS.md", "CLAUDE.md", "README.md"] {
        if let Ok(t) = std::fs::read_to_string(lib.join(f)) {
            return t.chars().take(12_000).collect();
        }
    }
    String::new()
}

/// Today's daily note and its template, from Obsidian's own settings.
pub fn daily(lib: &Path) -> (Option<String>, Option<String>) {
    let conf: Value = std::fs::read_to_string(lib.join(".obsidian").join("daily-notes.json")).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Value::Null);
    if conf.is_null() && !lib.join(".obsidian").is_dir() {
        return (None, None);
    }
    let folder = conf["folder"].as_str().unwrap_or("").trim_matches('/').to_string();
    if !lib.join(&folder).is_dir() {
        return (None, None);
    }
    let format = conf["format"].as_str().filter(|f| !f.is_empty()).unwrap_or("YYYY-MM-DD");
    let name = Local::now().format(&moment(format)).to_string();
    let path = if folder.is_empty() { format!("{name}.md") } else { format!("{folder}/{name}.md") };
    let template = conf["template"].as_str().filter(|t| !t.is_empty()).map(|t| if t.ends_with(".md") { t.to_string() } else { format!("{t}.md") });
    (Some(path), template)
}

pub fn plan(s: &Settings, notes: &[Note]) -> Result<Vec<Step>, String> {
    if notes.is_empty() {
        return Ok(Vec::new());
    }
    let lib = library(s).ok_or("Choose the folder your notes get sorted into, in Settings")?;
    let (daily_note, _) = daily(&lib);
    let items: Vec<Value> = notes
        .iter()
        .map(|n| {
            let at = Local.timestamp_millis_opt(n.created).single().map(|t| t.format("%Y-%m-%d %H:%M").to_string()).unwrap_or_default();
            json!({ "id": n.id, "written": at, "text": n.text, "app": n.app, "window": n.title })
        })
        .collect();
    let user = json!({
        "today": Local::now().format("%Y-%m-%d %A %H:%M").to_string(),
        "daily_note": daily_note,
        "library_rules": rules(&lib),
        "library_map": map(&lib, &crate::store::root()),
        "notes": items
    })
    .to_string();
    let raw = ask(s, SYSTEM, &user, &schema())?;
    let parsed = json_in(&raw).ok_or("The model answered in a shape Tuck can't read")?;
    let steps: Vec<Step> = serde_json::from_value(parsed["items"].clone()).map_err(|_| "The model answered in a shape Tuck can't read")?;
    Ok(steps
        .into_iter()
        .filter(|st| notes.iter().any(|n| n.id == st.id))
        .map(|mut st| {
            if st.action != "keep" && (target(&lib, &st.path).is_none() || st.text.trim().is_empty()) {
                st.action = "keep".into();
            }
            st
        })
        .collect())
}

/// A path inside the library, or nothing. Never escapes the root.
pub fn target(lib: &Path, rel: &str) -> Option<PathBuf> {
    let rel = Path::new(rel.trim().trim_start_matches(['/', '\\']));
    if rel.as_os_str().is_empty() || rel.extension().is_none_or(|e| e != "md") {
        return None;
    }
    if rel.components().any(|c| !matches!(c, Component::Normal(_))) {
        return None;
    }
    // No dot folders, no stream names (`a.md:x`), no names Windows reserves or trims.
    const RESERVED: &[&str] = &["con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8", "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9"];
    let bad = |c: Component| {
        let name = c.as_os_str().to_string_lossy();
        let stem = name.split('.').next().unwrap_or("").trim().to_lowercase();
        name.starts_with('.') || name.ends_with([' ', '.']) || name.chars().any(|ch| "<>:\"|?*".contains(ch) || ch.is_control()) || RESERVED.contains(&stem.as_str())
    };
    if rel.components().any(bad) {
        return None;
    }
    Some(lib.join(rel))
}

/// Writes one step into the library. Returns the path it wrote, relative.
pub fn write(lib: &Path, st: &Step) -> Result<String, String> {
    let path = target(lib, &st.path).ok_or("That place is outside the library")?;
    // Only existing folders: a made-up or renamed folder must not appear in the library.
    if !path.parent().is_some_and(Path::is_dir) {
        return Err("That folder doesn't exist".into());
    }
    let text = format!("{}\n", st.text.trim_end());
    let rel = |p: &Path| p.strip_prefix(lib).unwrap_or(p).to_string_lossy().replace('\\', "/");
    if st.action == "create" {
        let stem = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
        let mut free = path.clone();
        let mut i = 2;
        while free.exists() {
            free = path.with_file_name(format!("{stem} {i}.md"));
            i += 1;
        }
        std::fs::write(&free, text).map_err(|e| e.to_string())?;
        return Ok(rel(&free));
    }
    let mut body = if path.exists() {
        std::fs::read_to_string(&path).map_err(|e| e.to_string())?
    } else {
        fresh_daily(lib, &st.path)
    };
    let crlf = body.contains("\r\n");
    body = body.replace("\r\n", "\n");
    body = insert(&body, st.under.trim(), &text);
    if crlf {
        body = body.replace('\n', "\r\n");
    }
    let tmp = path.with_extension("md.tuck");
    std::fs::write(&tmp, body).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
    Ok(rel(&path))
}

/// A missing daily note starts from the vault's daily template.
fn fresh_daily(lib: &Path, rel: &str) -> String {
    let (daily_note, template) = daily(lib);
    if daily_note.as_deref() != Some(rel.trim()) {
        return String::new();
    }
    let Some(t) = template.and_then(|t| std::fs::read_to_string(lib.join(t)).ok()) else { return String::new() };
    let today = Local::now();
    let mut out = String::new();
    let mut rest = t.as_str();
    while let Some(i) = rest.find("{{") {
        out.push_str(&rest[..i]);
        let Some(j) = rest[i..].find("}}") else { break };
        let token = rest[i + 2..i + j].trim();
        let value = match token.split_once(':') {
            Some(("date" | "time", f)) => today.format(&moment(f.trim())).to_string(),
            None if token == "date" => today.format("%Y-%m-%d").to_string(),
            None if token == "time" => today.format("%H:%M").to_string(),
            None if token == "title" => Path::new(rel).file_stem().unwrap_or_default().to_string_lossy().to_string(),
            _ => rest[i..i + j + 2].to_string(),
        };
        out.push_str(&value);
        rest = &rest[i + j + 2..];
    }
    out.push_str(rest);
    out
}

/// Adds text at the end of a heading's section, or at the end of the file.
pub fn insert(body: &str, under: &str, text: &str) -> String {
    let lines: Vec<&str> = body.split('\n').collect();
    // Heading level per line: "## x" is a heading, "#tag" and lines inside code fences are not.
    let mut fenced = false;
    let levels: Vec<usize> = lines
        .iter()
        .map(|l| {
            if l.trim_start().starts_with("```") || l.trim_start().starts_with("~~~") {
                fenced = !fenced;
                return 0;
            }
            let n = l.chars().take_while(|c| *c == '#').count();
            let real = !fenced && (1..=6).contains(&n) && l[n..].chars().next().is_none_or(|c| c == ' ' || c == '\t');
            if real { n } else { 0 }
        })
        .collect();
    let want = under.trim_start_matches('#').trim();
    let head = (!want.is_empty()).then(|| (0..lines.len()).find(|&i| levels[i] > 0 && lines[i][levels[i]..].trim() == want)).flatten();
    let Some(h) = head else {
        let base = body.trim_end_matches('\n');
        return if base.is_empty() { text.to_string() } else { format!("{base}\n{text}") };
    };
    let lv = levels[h];
    let mut end = (h + 1..lines.len()).find(|&i| levels[i] > 0 && levels[i] <= lv).unwrap_or(lines.len());
    while end > h + 1 && lines[end - 1].trim().is_empty() {
        end -= 1;
    }
    let mut out: Vec<String> = lines[..end].iter().map(|s| s.to_string()).collect();
    out.extend(text.trim_end_matches('\n').split('\n').map(String::from));
    let rest = &lines[end..];
    if !rest.is_empty() && rest.iter().any(|l| !l.trim().is_empty()) && !rest[0].trim().is_empty() {
        out.push(String::new());
    }
    out.extend(rest.iter().map(|s| s.to_string()));
    let mut joined = out.join("\n");
    if !joined.ends_with('\n') {
        joined.push('\n');
    }
    joined
}

// ---------------------------------------------------------------- OpenRouter sign-in

/// OpenRouter's OAuth with PKCE: the browser signs in, OpenRouter hands back
/// a key made for Tuck. Waits up to five minutes for the browser.
pub fn openrouter_login() -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let mut raw = [0u8; 32];
    getrandom::fill(&mut raw).map_err(|e| e.to_string())?;
    let verifier = b64url(&raw);
    let challenge = b64url(&Sha256::digest(verifier.as_bytes()));
    let listener = std::net::TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let url = format!("https://openrouter.ai/auth?callback_url=http://localhost:{port}/tuck&code_challenge={challenge}&code_challenge_method=S256");
    open_url(&url);
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let start = Instant::now();
    let code = loop {
        match listener.accept() {
            Ok((mut stream, _)) => {
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                let code = req.split_whitespace().nth(1).and_then(|p| p.split_once("code=")).map(|(_, c)| c.split('&').next().unwrap_or("").to_string());
                let page = "<!doctype html><meta charset=utf-8><title>Tuck</title><body style=\"font:16px system-ui;display:grid;place-items:center;height:90vh\">Signed in. You can close this tab.";
                let _ = stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}", page.len()).as_bytes());
                if let Some(c) = code.filter(|c| !c.is_empty()) {
                    break c;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if start.elapsed() > Duration::from_secs(300) {
                    return Err("Sign-in timed out".into());
                }
                std::thread::sleep(Duration::from_millis(150));
            }
            Err(e) => return Err(e.to_string()),
        }
    };
    let mut res = http()
        .post("https://openrouter.ai/api/v1/auth/keys")
        .send_json(json!({ "code": code, "code_verifier": verifier, "code_challenge_method": "S256" }))
        .map_err(|e| format!("Can't reach OpenRouter: {e}"))?;
    let v: Value = res.body_mut().read_json().unwrap_or(Value::Null);
    v["key"].as_str().map(String::from).ok_or_else(|| "OpenRouter did not return a key".into())
}

fn b64url(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for i in 0..chunk.len() + 1 {
            out.push(A[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
    }
    out
}

pub fn open_url(url: &str) {
    let _ = hidden(Command::new("rundll32.exe").args(["url.dll,FileProtocolHandler", url])).spawn();
}

/// Opens a terminal where the person signs in to an agent CLI themselves.
pub fn cli_login(id: &str) -> Result<(), String> {
    let line = match id {
        "claude-code" => "claude",
        "codex" => "codex login",
        "gemini-cli" => "gemini",
        _ => return Err("Nothing to sign in to".into()),
    };
    Command::new("cmd").args(["/C", "start", "Sign in", "cmd", "/K", line]).spawn().map(|_| ()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Plans the real inbox with the real settings; writes nothing. `cargo test live_plan -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_plan() {
        let s = crate::settings::load();
        crate::store::set_root(&s.inbox);
        let notes: Vec<Note> = crate::store::list("deck").into_iter().filter(|n| n.pin.is_none()).collect();
        let steps = plan(&s, &notes).expect("plan");
        println!("{}", serde_json::to_string_pretty(&steps).unwrap());
    }

    /// A one-shot local HTTP server: answers each request with `reply` and
    /// hands back what it received (request line, headers, body), lowercased headers.
    fn fake(replies: Vec<(u16, Value)>) -> (String, std::sync::mpsc::Receiver<(String, String, Value)>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://127.0.0.1:{}/v1", listener.local_addr().unwrap().port());
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for (status, reply) in replies {
                let (mut s, _) = listener.accept().unwrap();
                let mut buf = Vec::new();
                let mut chunk = [0u8; 8192];
                let (head, body) = loop {
                    let n = s.read(&mut chunk).unwrap();
                    buf.extend_from_slice(&chunk[..n]);
                    let text = String::from_utf8_lossy(&buf).to_string();
                    if let Some(i) = text.find("\r\n\r\n") {
                        let head = text[..i].to_string();
                        let len = head.lines().find_map(|l| l.to_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap())).unwrap_or(0);
                        while buf.len() < i + 4 + len {
                            let n = s.read(&mut chunk).unwrap();
                            buf.extend_from_slice(&chunk[..n]);
                        }
                        break (head, String::from_utf8_lossy(&buf[i + 4..i + 4 + len]).to_string());
                    }
                };
                let line = head.lines().next().unwrap_or("").to_string();
                let body: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
                tx.send((line, head.to_lowercase(), body)).unwrap();
                let out = reply.to_string();
                let _ = s.write_all(format!("HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{out}", out.len()).as_bytes());
            }
        });
        (base, rx)
    }

    fn temp_lib(name: &str) -> PathBuf {
        let lib = std::env::temp_dir().join(format!("tuck-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&lib);
        std::fs::create_dir_all(lib.join("Tasks")).unwrap();
        std::fs::create_dir_all(lib.join("Ideas")).unwrap();
        std::fs::write(lib.join("Tasks").join("Work.md"), "# Work\r\n\r\n## Inbox\r\n- [ ] old\r\n\r\n## Done\r\n").unwrap();
        lib
    }

    fn note(id: &str, text: &str) -> Note {
        Note { id: id.into(), created: 1_700_000_000_000, text: text.into(), ..Default::default() }
    }

    /// Sort through an OpenAI-compatible API key provider against a local fake
    /// server, with a temp folder as the library: only safe paths are written.
    #[test]
    fn sort_with_key_provider_and_temp_library() {
        let lib = temp_lib("openai");
        let answer = json!({ "items": [
            { "id": "1", "action": "append", "path": "Tasks/Work.md", "under": "## Inbox", "text": "- [ ] call the bank", "why": "work tasks" },
            { "id": "2", "action": "create", "path": "Ideas/Garden.md", "under": "", "text": "Plant tulips in March", "why": "ideas" },
            { "id": "3", "action": "create", "path": "../escape.md", "under": "", "text": "x", "why": "" },
            { "id": "4", "action": "append", "path": ".obsidian/app.md", "under": "", "text": "x", "why": "" },
            { "id": "5", "action": "create", "path": "Nope/x.md", "under": "", "text": "x", "why": "" },
            { "id": "9", "action": "create", "path": "Ideas/Ghost.md", "under": "", "text": "not a note we sent", "why": "" }
        ]});
        let (base, rx) = fake(vec![(200, json!({ "choices": [{ "message": { "content": format!("Here you go:\n```json\n{answer}\n```") } }] }))]);
        let s = Settings { ai_provider: "openai".into(), ai_url: base, ai_model: "test-model".into(), ai_key: crate::settings::seal("sk-test"), library: lib.display().to_string(), ..Default::default() };
        let notes: Vec<Note> = (1..=5).map(|i| note(&i.to_string(), "text")).collect();
        let steps = plan(&s, &notes).expect("plan");

        let (line, head, body) = rx.recv().unwrap();
        assert_eq!(line, "POST /v1/chat/completions HTTP/1.1");
        assert!(head.contains("authorization: bearer sk-test"));
        assert_eq!(body["model"], "test-model");
        assert_eq!(body["response_format"]["type"], "json_object");
        let sent: Value = serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
        assert!(sent["library_map"].as_str().unwrap().contains("Tasks/Work.md"));
        assert_eq!(sent["notes"].as_array().unwrap().len(), 5);

        // The unknown id is dropped, unsafe paths become "keep".
        let action = |id: &str| steps.iter().find(|s| s.id == id).map(|s| s.action.clone());
        assert_eq!(action("9"), None);
        assert_eq!(action("3").as_deref(), Some("keep"));
        assert_eq!(action("4").as_deref(), Some("keep"));
        for st in steps.iter().filter(|s| s.action != "keep") {
            let r = write(&lib, st);
            if st.id == "5" {
                assert!(r.is_err(), "a made-up folder must not be created");
            } else {
                r.expect("write");
            }
        }
        let work = std::fs::read_to_string(lib.join("Tasks").join("Work.md")).unwrap();
        assert_eq!(work, "# Work\r\n\r\n## Inbox\r\n- [ ] old\r\n- [ ] call the bank\r\n\r\n## Done\r\n");
        assert_eq!(std::fs::read_to_string(lib.join("Ideas").join("Garden.md")).unwrap(), "Plant tulips in March\n");
        assert!(!lib.join("Nope").exists());
        assert!(!lib.parent().unwrap().join("escape.md").exists());
        // Creating the same note twice never overwrites.
        let again = Step { id: "2".into(), action: "create".into(), path: "Ideas/Garden.md".into(), under: String::new(), text: "second".into(), why: String::new() };
        assert_eq!(write(&lib, &again).unwrap(), "Ideas/Garden 2.md");
        let _ = std::fs::remove_dir_all(&lib);
    }

    /// The Anthropic path against a local fake server: headers, structured output, errors.
    #[test]
    fn anthropic_provider_against_fake_server() {
        let ok = json!({ "content": [{ "type": "text", "text": "{\"ok\": true}" }], "stop_reason": "end_turn" });
        let (base, rx) = fake(vec![(200, ok), (401, json!({ "error": { "message": "bad key" } })), (200, json!({ "content": [], "stop_reason": "refusal" }))]);
        let s = Settings { ai_provider: "anthropic".into(), ai_url: base, ai_key: crate::settings::seal("sk-ant-test"), ..Default::default() };
        test(&s).expect("connection test");
        let (line, head, body) = rx.recv().unwrap();
        assert_eq!(line, "POST /v1/messages HTTP/1.1");
        assert!(head.contains("x-api-key: sk-ant-test"));
        assert!(head.contains("anthropic-version:"));
        assert!(head.contains("anthropic-beta: server-side-fallback"));
        assert_eq!(body["model"], "claude-opus-5-5");
        assert_eq!(body["output_config"]["format"]["type"], "json_schema");
        assert_eq!(test(&s).unwrap_err(), "Anthropic rejected the API key");
        assert_eq!(test(&s).unwrap_err(), "The model declined to sort these notes");
        // No key, no request.
        let none = Settings { ai_key: String::new(), ..s };
        assert!(test(&none).unwrap_err().contains("API key"));
    }

    /// Model lists from /models, and a local server without a key.
    #[test]
    fn models_and_local_server() {
        let list = json!({ "data": [{ "id": "llama-3" }, { "id": "text-embed-1" }, { "id": "qwen" }] });
        let chat = json!({ "choices": [{ "message": { "content": "{\"ok\": true}" } }] });
        let (base, rx) = fake(vec![(200, list), (200, chat)]);
        assert_eq!(models("custom", &base, "").unwrap(), vec!["llama-3", "qwen"]);
        let (line, head, _) = rx.recv().unwrap();
        assert_eq!(line, "GET /v1/models HTTP/1.1");
        assert!(!head.contains("authorization"));
        let s = Settings { ai_provider: "custom".into(), ai_url: base, ai_model: "qwen".into(), ..Default::default() };
        test(&s).expect("custom server");
        let (_, head, body) = rx.recv().unwrap();
        assert!(!head.contains("authorization"));
        assert!(body.get("response_format").is_none());
    }

    #[test]
    fn insert_under_heading_and_paths() {
        let body = "# Day\n\n## Activity\n- 09:00 a\n\n## Notes\nx\n";
        let out = insert(body, "## Activity", "- 10:00 b\n");
        assert_eq!(out, "# Day\n\n## Activity\n- 09:00 a\n- 10:00 b\n\n## Notes\nx\n");
        assert_eq!(insert("x\n", "", "y\n"), "x\ny\n");
        assert_eq!(insert("", "## Nope", "y\n"), "y\n");
        // An Obsidian tag at the start of a line and a comment inside code are not headings.
        let tagged = "## Activity\n#idea a\n```sh\n# comment\n```\n## Notes\n";
        assert_eq!(insert(tagged, "## Activity", "b\n"), "## Activity\n#idea a\n```sh\n# comment\n```\nb\n\n## Notes\n");
        let lib = Path::new("C:/lib");
        assert!(target(lib, "02 Tasks/Work.md").is_some());
        assert!(target(lib, "../evil.md").is_none());
        assert!(target(lib, ".obsidian/x.md").is_none());
        assert!(target(lib, "C:/Windows/x.md").is_none());
        assert!(target(lib, "notes.txt").is_none());
        assert!(target(lib, "Work.md:hidden.md").is_none());
        assert!(target(lib, "Tasks/CON.md").is_none());
        assert!(target(lib, "Tasks /x.md").is_none());
        // A UNC-looking path loses its leading slashes and lands inside the library.
        assert!(target(lib, "\\\\server\\share\\x.md").is_none_or(|p| p.starts_with(lib)));
        assert!(target(lib, "Tasks/../../x.md").is_none());
        assert_eq!(b64url(&[0xfb, 0xff]), "-_8");
    }
}
