import type { Dest, Note, Settings } from "./api";

type Child = Node | string | null | undefined | false;
type TagOf<S extends string> = S extends `${infer T}.${string}` ? T : S;
type El<S extends string> = TagOf<S> extends keyof HTMLElementTagNameMap ? HTMLElementTagNameMap[TagOf<S>] : HTMLElement;

/** Tiny element builder: h("div.card.big", { onclick }, child, ...). */
export function h<S extends string>(tag: S, props: Record<string, unknown> = {}, ...kids: (Child | Child[])[]): El<S> {
  const [name, ...classes] = tag.split(".");
  const el = document.createElement(name) as El<S>;
  if (classes.length) el.className = classes.join(" ");
  for (const [k, v] of Object.entries(props)) {
    if (v === undefined || v === null || v === false) continue;
    if (k.startsWith("on")) el.addEventListener(k.slice(2), v as EventListener);
    else if (k === "html") el.innerHTML = v as string;
    else if (k === "style") el.setAttribute("style", v as string);
    else if (k in el && k !== "list") (el as unknown as Record<string, unknown>)[k] = v;
    else el.setAttribute(k, v === true ? "" : String(v));
  }
  for (const kid of kids.flat()) if (kid) el.append(kid);
  return el;
}

const paths: Record<string, string> = {
  send: '<path d="M7 17 17 7M8 7h9v9"/>',
  pin: '<path d="M9 4h6l-1 6 3 3v2H7v-2l3-3z"/><path d="M12 15v5"/>',
  copy: '<rect x="8" y="8" width="12" height="12" rx="2.5"/><path d="M16 8V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v8a2 2 0 0 0 2 2h2"/>',
  trash: '<path d="M4 7h16M10 11v6M14 11v6M6 7l1 12a2 2 0 0 0 2 2h6a2 2 0 0 0 2-2l1-12M9 7V4h6v3"/>',
  gear: '<circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z"/>',
  spark: '<path d="M12 3l1.8 5.2L19 10l-5.2 1.8L12 17l-1.8-5.2L5 10l5.2-1.8z"/><path d="M19 16l.7 2 2 .7-2 .7-.7 2-.7-2-2-.7 2-.7z"/>',
  search: '<circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5"/>',
  x: '<path d="M6 6l12 12M18 6 6 18"/>',
  check: '<path d="M5 12.5 10 17 19 7"/>',
  clock: '<circle cx="12" cy="12" r="8.5"/><path d="M12 7.5V12l3 2"/>',
  window: '<rect x="3.5" y="4.5" width="17" height="15" rx="2.5"/><path d="M3.5 9h17"/>',
  layers: '<path d="M12 4 3 9l9 5 9-5z"/><path d="m3 14 9 5 9-5"/>',
  float: '<rect x="7" y="3.5" width="13" height="11" rx="2.5"/><path d="M4 9v9a2 2 0 0 0 2 2h9"/>',
  back: '<path d="M15 5l-7 7 7 7"/>',
  folder: '<path d="M3.5 7.5a2 2 0 0 1 2-2h4l2 2.5h7a2 2 0 0 1 2 2v7.5a2 2 0 0 1-2 2h-13a2 2 0 0 1-2-2z"/>',
  plus: '<path d="M12 5v14M5 12h14"/>',
  undo: '<path d="M9 14 4 9l5-5"/><path d="M4 9h10.5a5.5 5.5 0 0 1 0 11H11"/>',
  clip: '<path d="M9 4h6v3H9z"/><path d="M15 5.5h2a2 2 0 0 1 2 2V19a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V7.5a2 2 0 0 1 2-2h2"/>',
  phone: '<path d="M5 4h4l2 5-2.5 1.5a11 11 0 0 0 5 5L15 13l5 2v4a2 2 0 0 1-2 2A16 16 0 0 1 3 6a2 2 0 0 1 2-2"/>',
  arrow: '<path d="M5 12h14M13 6l6 6-6 6"/>',
  inbox: '<path d="M3.5 13.5 6 5.5h12l2.5 8V18a2 2 0 0 1-2 2h-13a2 2 0 0 1-2-2z"/><path d="M3.5 13.5H8l1.5 2.5h5l1.5-2.5h4.5"/>',
  mouse: '<rect x="6.5" y="3" width="11" height="18" rx="5.5"/><path d="M12 7v3"/>',
};

export function icon(name: keyof typeof paths | string, size = 16): SVGSVGElement {
  const wrap = document.createElement("div");
  wrap.innerHTML = `<svg width="${size}" height="${size}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${paths[name] ?? ""}</svg>`;
  return wrap.firstElementChild as SVGSVGElement;
}

export const COLORS = ["butter", "rose", "sky"] as const;

export function applyTheme(theme?: Settings["theme"]) {
  const t = theme ?? (localStorage.getItem("tuck-theme") as Settings["theme"] | null) ?? "system";
  try {
    localStorage.setItem("tuck-theme", t);
  } catch {}
  if (t === "system") delete document.documentElement.dataset.theme;
  else document.documentElement.dataset.theme = t;
}

export function tags(text: string): string[] {
  const out: string[] = [];
  for (const m of text.matchAll(/(?:^|[\s,(])#([\p{L}\p{N}_\-/]+)/gu)) {
    const t = m[1].toLowerCase();
    if (!/^\d+$/.test(t) && !out.includes(t)) out.push(t);
  }
  return out;
}

/** Where a note would go right now, as the Rust side decides it. */
export function routeOf(s: Settings, text: string, to?: string | null): Dest | undefined {
  if (to) {
    const d = s.dests.find((x) => x.id === to);
    if (d) return d;
  }
  const t = tags(text);
  const rule = s.rules.find((r) => t.includes(r.tag.replace(/^#/, "").toLowerCase()));
  return (rule && s.dests.find((d) => d.id === rule.dest)) || s.dests.find((d) => d.id === s.default_dest);
}

export function destLabel(d: Dest): string {
  const where = d.kind === "notion" ? "page" : d.mode === "daily" ? "daily note" : d.mode === "inbox" ? d.file || "Inbox.md" : `${d.file || "Inbox"}/`;
  return `${d.name} · ${where}`;
}

const DAY = 86_400_000;
const weekdays = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];

/** "@15:00", "@3pm", "@20m", "@2h", "@tomorrow 9", "@fri" — the last one in the text wins. */
export function parseRemind(text: string, now = new Date()): { at: number; label: string } | null {
  const tokens = [...text.matchAll(/@(\S+)(?:\s+(\d{1,2}(?::\d{2})?\s?(?:am|pm)?))?/gi)];
  for (let i = tokens.length - 1; i >= 0; i--) {
    const word = tokens[i][1].toLowerCase().replace(/[.,;!?]+$/, "");
    const next = tokens[i][2];
    const rel = word.match(/^(\d+)(m|min|h|hr|d)$/);
    if (rel) {
      const n = +rel[1];
      const ms = rel[2].startsWith("m") ? n * 60_000 : rel[2].startsWith("h") ? n * 3_600_000 : n * DAY;
      if (n > 0) return { at: now.getTime() + ms, label: rel[2].startsWith("d") ? `in ${n} d` : `in ${n}${rel[2].startsWith("m") ? " min" : " h"}` };
      continue;
    }
    const day = new Date(now);
    let dayWord = "";
    let time = clock(word);
    if (!time) {
      if (word === "tomorrow" || word === "tmr" || word === "завтра") {
        day.setDate(day.getDate() + 1);
        dayWord = "tomorrow";
      } else if (weekdays.includes(word.slice(0, 3)) && /^[a-z]+$/.test(word)) {
        const target = weekdays.indexOf(word.slice(0, 3));
        const ahead = (target - day.getDay() + 7) % 7 || 7;
        day.setDate(day.getDate() + ahead);
        dayWord = word.slice(0, 1).toUpperCase() + word.slice(1, 3);
      } else continue;
      time = (next && clock(next.replace(/\s/g, ""))) || [9, 0];
    }
    day.setHours(time[0], time[1], 0, 0);
    if (!dayWord && day.getTime() <= now.getTime()) {
      day.setDate(day.getDate() + 1);
      dayWord = "tomorrow";
    }
    const hm = `${time[0]}:${String(time[1]).padStart(2, "0")}`;
    return { at: day.getTime(), label: dayWord ? `${dayWord} ${hm}` : hm };
  }
  return null;
}

function clock(s: string): [number, number] | null {
  const m = s.match(/^(\d{1,2})(?::(\d{2}))?(am|pm)?$/);
  if (!m) return null;
  let hr = +m[1];
  const min = m[2] ? +m[2] : 0;
  if (!m[2] && !m[3] && hr > 23) return null;
  if (m[3] === "pm" && hr < 12) hr += 12;
  if (m[3] === "am" && hr === 12) hr = 0;
  if (hr > 23 || min > 59) return null;
  // A bare "@3" is ambiguous; only times with ":" or am/pm, or 7..23 as hours.
  if (!m[2] && !m[3] && hr < 7) return null;
  return [hr, min];
}

export function ago(ms: number, now = Date.now()): string {
  const d = Math.max(0, now - ms);
  if (d < 60_000) return "now";
  if (d < 3_600_000) return `${Math.floor(d / 60_000)}m`;
  const then = new Date(ms);
  const today = new Date(now);
  const hm = then.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  if (then.toDateString() === today.toDateString()) return hm;
  if (now - ms < 6 * DAY) return `${then.toLocaleDateString([], { weekday: "short" })} ${hm}`;
  return then.toLocaleDateString([], { day: "numeric", month: "short" });
}

export function when(ms: number): string {
  const d = new Date(ms);
  const hm = d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  const today = new Date();
  const tomorrow = new Date(today.getTime() + DAY);
  if (d.toDateString() === today.toDateString()) return hm;
  if (d.toDateString() === tomorrow.toDateString()) return `tomorrow ${hm}`;
  return `${d.toLocaleDateString([], { weekday: "short", day: "numeric", month: "short" })} ${hm}`;
}

function esc(s: string): string {
  return s.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]!);
}

function inline(s: string): string {
  return esc(s)
    .replace(/`([^`]+)`/g, "<code>$1</code>")
    .replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>")
    .replace(/(^|[^*])\*([^*\s][^*]*)\*/g, "$1<em>$2</em>")
    .replace(/~~([^~]+)~~/g, "<s>$1</s>")
    .replace(/(https?:\/\/[^\s<]+[^\s<.,;:!?)])/g, '<a href="$1" data-link>$1</a>')
    .replace(/(^|\s)#([\p{L}\p{N}_\-/]+)/gu, '$1<span class="tag">#$2</span>')
    .replace(/(^|\s)(@\S+)/g, '$1<span class="when">$2</span>');
}

/** A small Markdown subset that suits notes; checkboxes carry their line number. */
export function render(text: string): string {
  const lines = text.split("\n");
  let out = "";
  let list = false;
  lines.forEach((raw, i) => {
    const task = raw.match(/^(\s*)- \[( |x|X)\] (.*)$/);
    const bullet = raw.match(/^(\s*)[-*] (.*)$/);
    const head = raw.match(/^(#{1,3}) (.*)$/);
    const isList = !!(task || bullet);
    if (isList && !list) out += "<ul>";
    if (!isList && list) out += "</ul>";
    list = isList;
    if (task) {
      const done = task[2] !== " ";
      out += `<li class="task${done ? " done" : ""}" style="margin-left:${task[1].length * 8}px"><button class="box" data-line="${i}" aria-label="toggle"></button><span>${inline(task[3])}</span></li>`;
    } else if (bullet) {
      out += `<li style="margin-left:${bullet[1].length * 8}px">${inline(bullet[2])}</li>`;
    } else if (head) {
      out += `<h${head[1].length + 2}>${inline(head[2])}</h${head[1].length + 2}>`;
    } else if (raw.trim() === "") {
      out += '<div class="gap"></div>';
    } else {
      out += `<p>${inline(raw)}</p>`;
    }
  });
  if (list) out += "</ul>";
  return out;
}

export function toggleLine(text: string, line: number): string {
  const lines = text.split("\n");
  lines[line] = lines[line].replace(/- \[( |x|X)\]/, (m) => (m.includes(" ]") ? "- [x]" : "- [ ]"));
  return lines.join("\n");
}

export function firstLine(n: Note): string {
  return (n.text.split("\n").find((l) => l.trim()) ?? "").replace(/^\s*(#+\s*|[-*]\s+\[[ xX]\]\s+|[-*]\s+|\d+\.\s+)/, "");
}

/** Plain-language key names for display. */
export function keyLabel(k: string): string {
  return k
    .split("+")
    .map((p) => ({ Control: "Ctrl", CommandOrControl: "Ctrl", Super: "Win", Meta: "Win" })[p] ?? p)
    .join(" + ");
}

/** Keyboard event to a shortcut string the Rust side understands. */
export function shortcutFrom(e: KeyboardEvent): string | null {
  if (["Control", "Shift", "Alt", "Meta"].includes(e.key)) return null;
  const mods = [e.ctrlKey && "Control", e.altKey && "Alt", e.shiftKey && "Shift", e.metaKey && "Super"].filter(Boolean) as string[];
  if (!mods.length) return null;
  const code = e.code;
  const key = code.startsWith("Key") ? code.slice(3) : code.startsWith("Digit") ? code.slice(5) : code === "Space" ? "Space" : code === "Backquote" ? "Backquote" : code.startsWith("F") ? code : code;
  return [...mods, key].join("+");
}
