//! A skill that teaches coding agents (Claude Code, Codex, any agent that
//! reads ~/.agents/skills) where Tuck keeps notes and how to sort them the
//! same way Tuck's own Sort does. It is written from the live settings and
//! rewritten whenever they change, so the paths in it are always true.

use crate::settings::Settings;
use crate::store::{folder, root};
use std::path::PathBuf;

fn text(s: &Settings) -> String {
    let inbox = root();
    let sent = folder("sent");
    let library = crate::ai::library(s).map(|p| p.display().to_string());
    let lib_line = match &library {
        Some(l) => format!("- Library (where notes get sorted into): `{l}`"),
        None => "- Library: not chosen yet. Ask the person where notes belong before sorting.".to_string(),
    };
    let rules_line = match &library {
        Some(l) => format!(
            "Before sorting, read the library's own rules for agents: `{l}\\AGENTS.md` (or `CLAUDE.md`, `README.md`). They win over everything below. Look at the folder layout and a few existing notes to match how things are written there."
        ),
        None => String::new(),
    };
    format!(
        r#"---
name: tuck
description: Find, add to, and sort the person's quick notes captured with Tuck (their inbox of unsorted thoughts, tasks and reminders). Use when they say "tuck this", "note that", "what's in my inbox", "sort my notes", "file my notes", or ask to remember something for later.
---

# Tuck

Tuck is a quick-note app. Every note is one Markdown file.

- Inbox (waiting, unsorted notes): `{inbox}`
- Sorted and sent notes, kept as a record: `{sent}`
{lib_line}

## A note file

One file per note in the inbox, named `YYYY-MM-DD HH.MM First words.md`. Front matter is optional:

```
---
id: 20261103-140212-123
created: 2026-11-03T14:02:12+01:00
app: VS Code
remind: 2026-11-03T15:00:00+01:00
---
The note text. #tags hint at a place. Checklists use - [ ] items.
```

Other keys you may see: `color` (paper colour, meaningless for sorting), `title` (window title it was written in), `pin` (JSON: the note is a sticky on the person's screen; never sort or move it), `to`, `sent`, `sent_at`. Files without `id` were written by hand or by an agent; their id is the file name.

## Add a note

Write a new file into the inbox. A bare body without front matter is fine. Write to `<name>.md.tmp` first and rename it, so Tuck never reads half a file. Tuck shows it within two seconds.

## Sort the inbox

{rules_line}

For each note in the inbox that has no `pin`:

1. Decide where it belongs:
   - a task goes to the matching task list, in that list's task format (keep a due time from `remind` or `@15:00` as a date in the library's format);
   - a fact about a project goes to that project's notes;
   - a thought, log line or anything about today goes to today's daily note, in its style;
   - a longer piece that stands alone becomes a new note in the folder where such notes live;
   - when unsure or the note is unfinished, leave it in the inbox.
2. Write it there. Append or create; never overwrite, delete, reorder or reword what is already in the library. Keep the person's words and language, never translate or summarise them.
3. Move the note file from the inbox to the sorted folder and add to its front matter `sent: <path you wrote, relative to the library>` and `sent_at: <now, RFC 3339>`.

Never delete notes. When you are done, say in one line per note where it went.
"#,
        inbox = inbox.display(),
        sent = sent.display(),
    )
}

fn homes() -> Vec<PathBuf> {
    let home = PathBuf::from(std::env::var("USERPROFILE").unwrap_or_default());
    [".claude", ".codex", ".agents"].iter().map(|a| home.join(a)).filter(|p| p.is_dir()).collect()
}

/// Writes the skill for every agent home that exists. Returns where it went.
pub fn install(s: &Settings) -> Result<Vec<String>, String> {
    let mut done = Vec::new();
    let body = text(s);
    for base in homes() {
        let dir = base.join("skills").join("tuck");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let file = dir.join("SKILL.md");
        if std::fs::read_to_string(&file).ok().as_deref() != Some(body.as_str()) {
            std::fs::write(&file, &body).map_err(|e| e.to_string())?;
        }
        done.push(dir.display().to_string());
    }
    if done.is_empty() {
        return Err("No Claude Code, Codex or other agent found in your home folder".into());
    }
    Ok(done)
}

/// Agent names Tuck can teach, for the setup screen.
pub fn agents() -> Vec<String> {
    homes()
        .iter()
        .filter_map(|p| match p.file_name()?.to_str()? {
            ".claude" => Some("Claude Code".to_string()),
            ".codex" => Some("Codex".to_string()),
            _ => None,
        })
        .collect()
}
