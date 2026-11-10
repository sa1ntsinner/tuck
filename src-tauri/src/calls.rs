//! Which call apps hold the microphone right now. Windows records every
//! app's microphone use for the privacy page; an entry whose stop time is
//! zero is in use. No audio is touched.

use windows::Win32::System::Registry::*;
use windows::core::{PCWSTR, w};

const STORE: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\CapabilityAccessManager\\ConsentStore\\microphone");

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn subkeys(key: HKEY) -> Vec<String> {
    let mut out = Vec::new();
    for i in 0.. {
        let mut name = [0u16; 512];
        let mut len = name.len() as u32;
        let r = unsafe { RegEnumKeyExW(key, i, Some(windows::core::PWSTR(name.as_mut_ptr())), &mut len, None, None, None, None) };
        if r.is_err() {
            break;
        }
        out.push(String::from_utf16_lossy(&name[..len as usize]));
    }
    out
}

fn in_use(parent: HKEY, name: &str) -> bool {
    let mut stop = 0u64;
    let mut start = 0u64;
    let mut size = 8u32;
    let n = wide(name);
    unsafe {
        let a = RegGetValueW(parent, PCWSTR(n.as_ptr()), w!("LastUsedTimeStop"), RRF_RT_REG_QWORD, None, Some(&mut stop as *mut _ as _), Some(&mut size));
        size = 8;
        let b = RegGetValueW(parent, PCWSTR(n.as_ptr()), w!("LastUsedTimeStart"), RRF_RT_REG_QWORD, None, Some(&mut start as *mut _ as _), Some(&mut size));
        a.is_ok() && b.is_ok() && stop == 0 && start != 0
    }
}

/// Identifiers (package names or exe paths with '#') of everything using the mic.
pub fn mic_users() -> Vec<String> {
    let mut out = Vec::new();
    unsafe {
        let mut root = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, STORE, None, KEY_READ, &mut root).is_err() {
            return out;
        }
        for name in subkeys(root) {
            if name == "NonPackaged" {
                let mut np = HKEY::default();
                if RegOpenKeyExW(root, w!("NonPackaged"), None, KEY_READ, &mut np).is_ok() {
                    out.extend(subkeys(np).into_iter().filter(|n| in_use(np, n)));
                    let _ = RegCloseKey(np);
                }
            } else if in_use(root, &name) {
                out.push(name);
            }
        }
        let _ = RegCloseKey(root);
    }
    out.into_iter().map(|s| s.to_lowercase()).collect()
}

/// A call app's name for a mic user, or None for anything else.
pub fn call_app(id: &str) -> Option<&'static str> {
    let apps = [
        ("zoom", "Zoom"),
        ("msteams", "Teams"),
        ("ms-teams", "Teams"),
        ("teams.exe", "Teams"),
        ("slack", "Slack"),
        ("discord", "Discord"),
        ("webex", "Webex"),
        ("ciscocollab", "Webex"),
        ("telegram", "Telegram"),
        ("whatsapp", "WhatsApp"),
        ("skype", "Skype"),
        ("signal", "Signal"),
        ("facetime", "FaceTime"),
        ("chrome.exe", "Browser"),
        ("msedge.exe", "Browser"),
        ("firefox.exe", "Browser"),
        ("brave.exe", "Browser"),
        ("arc.exe", "Browser"),
        ("helium.exe", "Browser"),
        ("vivaldi.exe", "Browser"),
        ("opera.exe", "Browser"),
    ];
    if id.contains("tuck") {
        return None;
    }
    apps.iter().find(|(k, _)| id.contains(k)).map(|(_, v)| *v)
}

pub fn active_calls() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = mic_users().iter().filter_map(|u| call_app(u)).collect();
    v.sort();
    v.dedup();
    v
}

#[cfg(test)]
mod tests {
    #[test]
    fn names() {
        assert_eq!(super::call_app("c:#program files#zoom#bin#zoom.exe"), Some("Zoom"));
        assert_eq!(super::call_app("msteams_8wekyb3d8bbwe"), Some("Teams"));
        assert_eq!(super::call_app("c:#apps#obs64.exe"), None);
    }
}
