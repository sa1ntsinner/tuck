import { invoke } from "@tauri-apps/api/core";

export interface Pin {
  mode: "float" | "desk" | "app";
  app: string;
  x: number;
  y: number;
  w: number;
  h: number;
  zoom: number;
}

export interface Note {
  id: string;
  created: number;
  text: string;
  color: string;
  app: string;
  title: string;
  pin: Pin | null;
  remind: number | null;
  to: string | null;
  sent: string | null;
  sent_at: number | null;
}

export type AppKind = "obsidian" | "logseq" | "folder" | "joplin" | "anytype" | "notion" | "todoist" | "tana";

export interface Found {
  kind: AppKind;
  name: string;
  path: string;
}

export interface Dest {
  id: string;
  name: string;
  kind: AppKind;
  path: string;
  mode: "daily" | "inbox" | "note";
  file: string;
  page: string;
  token: string;
}

export interface Rule {
  tag: string;
  dest: string;
}

export interface Settings {
  capture_key: string;
  sticky_key: string;
  deck_key: string;
  middle_hold: boolean;
  hold_ms: number;
  note_w: number;
  note_h: number;
  excluded: string[];
  dests: Dest[];
  default_dest: string;
  rules: Rule[];
  keep_source: boolean;
  ai_provider: string;
  ai_url: string;
  ai_model: string;
  ai_key: string;
  calls: boolean;
  quiet_apps: string[];
  theme: "system" | "light" | "dark";
  seen_intro: boolean;
  inbox: string;
  library: string;
  sort_auto: "off" | "hourly" | "evening";
  skill_auto: boolean;
}

export interface Provider {
  id: string;
  name: string;
  kind: "cli" | "key" | "local" | "custom";
  ready: boolean;
  state: string;
}

export interface SortState {
  ready: boolean;
  library: string | null;
  inbox: string;
  inbox_ok: boolean;
  agents: string[];
}

export interface Ctx {
  mode: "note" | "sticky" | "call";
  app: string;
  exe: string;
  title: string;
  clip: string;
  call: string;
  by_mouse: boolean;
}

export interface Step {
  id: string;
  action: "append" | "create" | "keep";
  path: string;
  under: string;
  text: string;
  why: string;
}

export const api = {
  ctx: () => invoke<Ctx>("ctx"),
  notes: (scope: "deck" | "sent" | "trash") => invoke<Note[]>("notes", { scope }),
  note: (id: string) => invoke<Note | null>("note", { id }),
  save: (note: Partial<Note>, action: "tuck" | "send" | "pin" | "pin_app") => invoke<Note>("save", { note: blank(note), action }),
  update: (note: Note) => invoke<void>("update", { note }),
  remove: (id: string) => invoke<void>("remove", { id }),
  restore: (id: string) => invoke<void>("restore", { id }),
  send: (id: string, dest?: string) => invoke<string>("send", { id, dest: dest ?? null }),
  sendAll: () => invoke<{ sent: number; kept: number; errors: string[] }>("send_all"),
  pin: (id: string, mode: "" | "float" | "desk" | "app") => invoke<void>("pin", { id, mode }),
  stickyGeom: (id: string, w: number, h: number, zoom: number, color?: string) =>
    invoke<void>("sticky_geom", { id, w, h, zoom, color: color ?? null }),
  settings: () => invoke<Settings>("get_settings"),
  setSettings: (s: Settings) => invoke<{ failed: string[] }>("set_settings", { s }),
  vaults: () => invoke<{ name: string; path: string }[]>("vaults"),
  foundApps: () => invoke<Found[]>("found_apps"),
  anytypeChallenge: () => invoke<string>("anytype_challenge"),
  notionPages: (token: string) => invoke<{ id: string; title: string }[]>("notion_pages", { token }),
  /** Adds the app to the places; returns its name. */
  connect: (kind: AppKind, a = "", b = "") => invoke<string>("connect", { kind, a, b }),
  sortPlan: () => invoke<Step[]>("sort_plan"),
  sortApply: (steps: Step[]) => invoke<{ done: number; errors: string[] }>("sort_apply", { steps }),
  sortState: () => invoke<SortState>("sort_state"),
  providers: () => invoke<Provider[]>("providers"),
  models: (provider: string, url: string, key: string) => invoke<string[]>("models", { provider, url, key }),
  openrouterLogin: () => invoke<void>("openrouter_login"),
  cliLogin: (provider: string) => invoke<void>("cli_login", { provider }),
  testAi: () => invoke<void>("test_ai"),
  openUrl: (url: string) => invoke<void>("open_url", { url }),
  dismiss: () => invoke<void>("dismiss"),
  hide: () => invoke<void>("hide_window"),
  openDeck: () => invoke<void>("open_deck"),
  newCapture: (mode: "note" | "sticky") => invoke<void>("new_capture", { mode }),
  reveal: (what: "notes" | "settings") => invoke<void>("reveal", { what }),
  installSkill: () => invoke<string[]>("install_skill"),
  autostart: (on?: boolean) => invoke<boolean>("autostart", { on: on ?? null }),
  toastAction: (kind: string, action: string, id: string) => invoke<void>("toast_action", { kind, action, id }),
};

function blank(n: Partial<Note>): Note {
  return {
    id: "",
    created: 0,
    text: "",
    color: "",
    app: "",
    title: "",
    pin: null,
    remind: null,
    to: null,
    sent: null,
    sent_at: null,
    ...n,
  };
}

export function errorText(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : "Something went wrong";
}
