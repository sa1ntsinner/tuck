//! Settings live in %APPDATA%\Tuck\settings.json. Tokens and API keys are
//! sealed with DPAPI, so the file is useless on another account or PC.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use windows::Win32::Foundation::{HLOCAL, LocalFree};
use windows::Win32::Security::Cryptography::{CRYPT_INTEGER_BLOB, CryptProtectData, CryptUnprotectData};

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct Dest {
    pub id: String,
    pub name: String,
    /// "obsidian", "folder" or "notion".
    pub kind: String,
    pub path: String,
    /// "daily", "inbox" or "note".
    pub mode: String,
    /// Inbox file name, or the subfolder for new notes.
    pub file: String,
    pub page: String,
    /// Sealed. Sent to the UI as "" (none) or "•" (set); "•" coming back means keep.
    pub token: String,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct Rule {
    pub tag: String,
    pub dest: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Settings {
    pub capture_key: String,
    pub sticky_key: String,
    pub deck_key: String,
    pub middle_hold: bool,
    pub hold_ms: u64,
    /// Size the note panel opens at, in logical pixels (the window, shadow margin included).
    pub note_w: u32,
    pub note_h: u32,
    pub excluded: Vec<String>,
    pub dests: Vec<Dest>,
    pub default_dest: String,
    pub rules: Vec<Rule>,
    pub keep_source: bool,
    /// An id from ai::PROVIDERS (API, local server or a signed-in agent CLI).
    pub ai_provider: String,
    pub ai_url: String,
    pub ai_model: String,
    pub ai_key: String,
    pub calls: bool,
    pub quiet_apps: Vec<String>,
    pub theme: String,
    pub seen_intro: bool,
    /// Where waiting notes live. Empty means Documents/Tuck.
    pub inbox: String,
    /// The folder (usually an Obsidian vault) that Sort files notes into.
    pub library: String,
    /// "off", "hourly" or "evening" (once a day after 20:00).
    pub sort_auto: String,
    /// Keep the agent skill installed and up to date with these settings.
    pub skill_auto: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            capture_key: "Alt+Q".into(),
            sticky_key: "Alt+W".into(),
            deck_key: "Alt+Shift+Q".into(),
            middle_hold: true,
            hold_ms: 320,
            note_w: 640,
            note_h: 170,
            excluded: ["blender.exe", "freecad.exe", "sldworks.exe", "acad.exe", "inventor.exe", "fusion360.exe", "unity.exe", "unrealeditor.exe"]
                .map(String::from)
                .to_vec(),
            dests: Vec::new(),
            default_dest: String::new(),
            rules: Vec::new(),
            keep_source: true,
            ai_provider: "anthropic".into(),
            ai_url: String::new(),
            ai_model: String::new(),
            ai_key: String::new(),
            calls: true,
            quiet_apps: Vec::new(),
            theme: "system".into(),
            seen_intro: false,
            inbox: String::new(),
            library: String::new(),
            sort_auto: "off".into(),
            skill_auto: false,
        }
    }
}

fn path() -> PathBuf {
    let dir = PathBuf::from(std::env::var("APPDATA").unwrap_or_default()).join("Tuck");
    let _ = std::fs::create_dir_all(&dir);
    dir.join("settings.json")
}

pub fn load() -> Settings {
    std::fs::read_to_string(path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub fn save(s: &Settings) {
    let tmp = path().with_extension("json.tmp");
    if std::fs::write(&tmp, serde_json::to_string_pretty(s).unwrap_or_default()).is_ok() {
        let _ = std::fs::rename(tmp, path());
    }
}

/// What the UI may see: secrets become a marker.
pub fn masked(s: &Settings) -> Settings {
    let mut m = s.clone();
    let mark = |v: &str| if v.is_empty() { String::new() } else { "•".to_string() };
    m.ai_key = mark(&s.ai_key);
    for d in &mut m.dests {
        d.token = mark(&d.token);
    }
    m
}

/// Takes settings from the UI: new secrets get sealed, "•" keeps the old one.
pub fn merge(old: &Settings, mut new: Settings) -> Settings {
    let keep = |incoming: &str, previous: &str| match incoming {
        "•" => previous.to_string(),
        "" => String::new(),
        plain => seal(plain),
    };
    new.ai_key = keep(&new.ai_key, &old.ai_key);
    for d in &mut new.dests {
        let previous = old.dests.iter().find(|o| o.id == d.id).map(|o| o.token.clone()).unwrap_or_default();
        d.token = keep(&d.token, &previous);
    }
    new
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len() / 2).filter_map(|i| u8::from_str_radix(s.get(i * 2..i * 2 + 2)?, 16).ok()).collect()
}

pub fn seal(plain: &str) -> String {
    let mut input = plain.as_bytes().to_vec();
    let blob_in = CRYPT_INTEGER_BLOB { cbData: input.len() as u32, pbData: input.as_mut_ptr() };
    let mut out = CRYPT_INTEGER_BLOB::default();
    unsafe {
        if CryptProtectData(&blob_in, None, None, None, None, 0, &mut out).is_err() {
            return String::new();
        }
        let sealed = hex(std::slice::from_raw_parts(out.pbData, out.cbData as usize));
        let _ = LocalFree(Some(HLOCAL(out.pbData as _)));
        sealed
    }
}

pub fn open(sealed: &str) -> String {
    let mut input = unhex(sealed);
    if input.is_empty() {
        return String::new();
    }
    let blob_in = CRYPT_INTEGER_BLOB { cbData: input.len() as u32, pbData: input.as_mut_ptr() };
    let mut out = CRYPT_INTEGER_BLOB::default();
    unsafe {
        if CryptUnprotectData(&blob_in, None, None, None, None, 0, &mut out).is_err() {
            return String::new();
        }
        let plain = String::from_utf8_lossy(std::slice::from_raw_parts(out.pbData, out.cbData as usize)).to_string();
        let _ = LocalFree(Some(HLOCAL(out.pbData as _)));
        plain
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn seal_round_trip() {
        let s = super::seal("secret_abc");
        assert!(!s.is_empty() && !s.contains("secret"));
        assert_eq!(super::open(&s), "secret_abc");
    }
}
