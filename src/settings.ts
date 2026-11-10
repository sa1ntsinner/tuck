import { open } from "@tauri-apps/plugin-dialog";
import { api, errorText, type Dest, type Settings } from "./api";
import { connectList } from "./connect";
import { providerPicker } from "./providers";
import { h, icon, keyLabel, shortcutFrom } from "./ui";

interface Hooks {
  saved: (s: Settings) => void;
  dialog: (open: boolean) => void;
  close: () => void;
  setup: () => void;
}

const modeName: Record<Dest["mode"], string> = {
  daily: "Daily note",
  inbox: "Inbox file",
  note: "New note each",
};

const appName: Record<Dest["kind"], string> = {
  obsidian: "Obsidian",
  logseq: "Logseq journal",
  folder: "Folder",
  joplin: "Joplin",
  anytype: "Anytype",
  notion: "Notion",
  todoist: "Todoist inbox",
  tana: "Tana inbox",
};

/** Kinds that write Markdown files and so have a write mode. */
const files = (d: Dest) => d.kind === "obsidian" || d.kind === "folder";

export function settingsPage(root: HTMLElement, s: Settings, hooks: Hooks) {
  let timer = 0;
  let openDest: string | null = null;
  let connecting = false;
  const connected = h("p.test.ok.connected");
  const status = h("p.status");

  const save = (repaint = false) => {
    clearTimeout(timer);
    timer = window.setTimeout(async () => {
      let r: { failed: string[] };
      try {
        r = await api.setSettings(s);
      } catch (e) {
        status.textContent = errorText(e);
        return;
      }
      status.textContent = r.failed.length
        ? `${r.failed.map(keyLabel).join(", ")} belongs to another app. Pick another.`
        : "";
      hooks.saved(s);
      const fresh = await api.settings();
      s.ai_key = fresh.ai_key;
      for (const d of fresh.dests) {
        const mine = s.dests.find((x) => x.id === d.id);
        if (mine) mine.token = d.token;
      }
      if (repaint) paint();
    }, 250);
  };

  const sw = (on: boolean, flip: (v: boolean) => void, label: string) => {
    const b = h(`button.switch${on ? ".on" : ""}`, {
      role: "switch",
      "aria-checked": String(on),
      "aria-label": label,
    });
    b.addEventListener("click", () => {
      on = !on;
      b.classList.toggle("on", on);
      b.setAttribute("aria-checked", String(on));
      flip(on);
    });
    return b;
  };

  const row = (label: string | Node, ...control: (Node | null)[]) =>
    h("div.row", {}, h("div.label", {}, label), ...control);

  const keycap = (get: () => string, set: (v: string) => void) => {
    const b = h("button.keycap", {}, keyLabel(get()) || "Off");
    b.addEventListener("click", () => {
      b.classList.add("listening");
      b.textContent = "Press keys";
      hooks.dialog(true);
      const onKey = (e: KeyboardEvent) => {
        e.preventDefault();
        e.stopPropagation();
        if (e.key === "Escape") return done();
        if (e.key === "Backspace" || e.key === "Delete") {
          set("");
          save();
          return done();
        }
        const sc = shortcutFrom(e);
        if (sc) {
          set(sc);
          save();
          done();
        }
      };
      const done = () => {
        removeEventListener("keydown", onKey, true);
        b.classList.remove("listening");
        b.textContent = keyLabel(get()) || "Off";
        hooks.dialog(false);
      };
      addEventListener("keydown", onKey, true);
      b.addEventListener("blur", done, { once: true });
    });
    return b;
  };

  const field = (
    value: string,
    placeholder: string,
    onchange: (v: string) => void,
    type = "text",
    cls = "",
  ) => {
    const f = h(`input.field${cls}`, {
      value,
      placeholder,
      type,
      spellcheck: false,
    });
    f.addEventListener("input", () => {
      onchange(f.value);
      save();
    });
    return f;
  };

  const select = (
    value: string,
    options: [string, string][],
    onchange: (v: string) => void,
  ) => {
    const f = h(
      "select.field",
      {},
      ...options.map(([v, label]) =>
        h("option", { value: v, selected: v === value }, label),
      ),
    );
    f.addEventListener("change", () => {
      onchange(f.value);
      save(true);
    });
    return f;
  };

  // Presets for the note panel; dragging its corner makes a custom size, which shows here as it is.
  const noteSize = () => {
    const sizes: [string, string][] = [
      ["520x150", "Small"],
      ["640x170", "Medium"],
      ["800x240", "Large"],
      ["960x360", "Huge"],
    ];
    const now = `${s.note_w}x${s.note_h}`;
    if (!sizes.some(([v]) => v === now))
      sizes.push([now, `${s.note_w} × ${s.note_h}`]);
    return select(now, sizes, (v) => {
      const [w, hgt] = v.split("x").map(Number);
      s.note_w = w;
      s.note_h = hgt;
    });
  };

  const browse = async (): Promise<string | null> => {
    hooks.dialog(true);
    try {
      const picked = await open({ directory: true });
      return typeof picked === "string" ? picked : null;
    } finally {
      hooks.dialog(false);
    }
  };

  const editor = (d: Dest) => {
    const box = h("div.editor");
    box.append(
      h(
        "label",
        {},
        "Name",
        field(d.name, "Name", (v) => (d.name = v), "text", ".wide"),
      ),
    );
    if (d.kind === "notion") {
      box.append(
        h(
          "label",
          {},
          "Page link",
          field(d.page, "notion.so/…", (v) => (d.page = v), "text", ".wide"),
        ),
        h(
          "label",
          {},
          "Integration token",
          field(
            "",
            d.token === "•" ? "Saved" : "ntn_…",
            (v) => (d.token = v || (d.token === "•" ? "•" : "")),
            "password",
            ".wide",
          ),
        ),
        h(
          "p.foot-note",
          { style: "margin:0" },
          "Share the page with your integration: ••• → Connections.",
        ),
      );
    } else if (files(d)) {
      const pathBtn = h(
        "button.btn",
        { title: d.path },
        icon("folder", 14),
        d.path ? d.path.split(/[\\/]/).pop() : "Choose",
      );
      pathBtn.addEventListener("click", async () => {
        const p = await browse();
        if (!p) return;
        d.path = p;
        if (!d.name) d.name = p.split(/[\\/]/).pop() ?? "Folder";
        save(true);
      });
      box.append(
        h(
          "div.two",
          {},
          h("label", {}, "Folder", pathBtn),
          h(
            "label",
            {},
            "Write to",
            select(
              d.mode,
              Object.entries(modeName) as [string, string][],
              (v) => (d.mode = v as Dest["mode"]),
            ),
          ),
        ),
      );
      if (d.mode === "inbox")
        box.append(
          h(
            "label",
            {},
            "File",
            field(d.file, "Inbox.md", (v) => (d.file = v), "text", ".wide"),
          ),
        );
      if (d.mode === "note")
        box.append(
          h(
            "label",
            {},
            "Subfolder",
            field(d.file, "Inbox", (v) => (d.file = v), "text", ".wide"),
          ),
        );
      if (d.mode === "daily" && d.kind === "folder")
        box.append(
          h(
            "label",
            {},
            "Subfolder",
            field(d.file, "None", (v) => (d.file = v), "text", ".wide"),
          ),
        );
    }
    return box;
  };

  const paint = async () => {
    const [auto, state] = await Promise.all([api.autostart(), api.sortState()]);
    root.replaceChildren();

    // Notes and sorting
    const pick = async (set: (p: string) => void) => {
      const p = await browse();
      if (!p) return;
      set(p);
      save(true);
    };
    const last = (p: string) =>
      p.split(/[\\/]/).filter(Boolean).slice(-2).join(" › ");
    root.append(
      h("h2", { id: "sorting" }, "Notes"),
      h(
        "div.group",
        {},
        row(
          "Wait in",
          h(
            "button.btn",
            { title: state.inbox, onclick: () => pick((p) => (s.inbox = p)) },
            last(state.inbox),
          ),
        ),
        row(
          "Sorted into",
          h(
            "button.btn",
            {
              title: state.library ?? "",
              onclick: () => pick((p) => (s.library = p)),
            },
            state.library
              ? state.library.split(/[\\/]/).filter(Boolean).pop()
              : "Choose",
          ),
        ),
        row(
          "Sort runs",
          select(
            s.sort_auto,
            [
              ["off", "When I press Sort"],
              ["hourly", "Every hour"],
              ["evening", "Every evening"],
            ],
            (v) => (s.sort_auto = v as Settings["sort_auto"]),
          ),
        ),
        row(
          "Notes folder",
          h("button.btn", { onclick: () => api.reveal("notes") }, "Open"),
        ),
      ),
      status,
      h("h2", {}, "Sorted by"),
      providerPicker(
        s,
        async () => {
          await api.setSettings(s);
          s.ai_key = (await api.settings()).ai_key;
          hooks.saved(s);
        },
        hooks.dialog,
      ),
    );

    root.append(
      h("h2", {}, "Shortcuts"),
      h(
        "div.group",
        {},
        row(
          "New note",
          keycap(
            () => s.capture_key,
            (v) => (s.capture_key = v),
          ),
        ),
        row(
          "New sticky",
          keycap(
            () => s.sticky_key,
            (v) => (s.sticky_key = v),
          ),
        ),
        row(
          "Deck",
          keycap(
            () => s.deck_key,
            (v) => (s.deck_key = v),
          ),
        ),
        row(
          "Hold middle button",
          sw(
            s.middle_hold,
            (v) => ((s.middle_hold = v), save()),
            "Hold middle button",
          ),
        ),
        s.middle_hold
          ? row(
              "Hold for",
              select(
                String(s.hold_ms),
                [
                  ["250", "0.25 s"],
                  ["320", "0.3 s"],
                  ["450", "0.45 s"],
                ],
                (v) => (s.hold_ms = +v),
              ),
            )
          : null,
        s.middle_hold
          ? row(
              "Not in",
              field(
                s.excluded.join(", "),
                "blender.exe",
                (v) =>
                  (s.excluded = v
                    .split(/[,\s]+/)
                    .map((x) => x.trim().toLowerCase())
                    .filter(Boolean)),
              ),
            )
          : null,
      ),
    );

    // Places
    const group = h("div.group");
    for (const d of s.dests) {
      const isDefault = s.default_dest === d.id;
      const radio = h(`button.radio${isDefault ? ".on" : ""}`, {
        title: "Default",
        "aria-label": `Default: ${d.name}`,
      });
      radio.addEventListener("click", () => {
        s.default_dest = isDefault ? "" : d.id;
        save(true);
      });
      const where = files(d) ? modeName[d.mode] : appName[d.kind];
      group.append(
        h(
          "div.row",
          {},
          radio,
          h(
            "span.mark",
            {},
            d.kind === "folder" ? icon("folder", 14) : appName[d.kind][0],
          ),
          h(
            "div.label",
            {},
            d.name || "Untitled",
            h("span.dim", {}, isDefault ? `${where} · default` : where),
          ),
          h(
            "button.ghost",
            {
              title: "Edit",
              "aria-label": "Edit",
              onclick: () => (
                (openDest = openDest === d.id ? null : d.id),
                paint()
              ),
            },
            icon(openDest === d.id ? "x" : "gear", 15),
          ),
          h(
            "button.ghost",
            {
              title: "Remove",
              "aria-label": "Remove",
              onclick: () => {
                s.dests = s.dests.filter((x) => x.id !== d.id);
                s.rules = s.rules.filter((r) => r.dest !== d.id);
                if (s.default_dest === d.id)
                  s.default_dest = s.dests[0]?.id ?? "";
                save(true);
              },
            },
            icon("trash", 15),
          ),
        ),
      );
      if (openDest === d.id) group.append(editor(d));
    }
    const adders = h(
      "div.adders",
      {},
      h(
        "button.btn",
        { onclick: () => ((connecting = !connecting), paint()) },
        icon(connecting ? "x" : "plus", 13),
        connecting ? "Close" : "Connect an app",
      ),
    );
    group.append(adders);
    const picker = connecting
      ? connectList({
          browse,
          hold: hooks.dialog,
          done: async (name) => {
            const fresh = await api.settings();
            s.dests = fresh.dests;
            s.default_dest = fresh.default_dest;
            connecting = false;
            connected.textContent = `Connected ${name}`;
            paint();
          },
        })
      : null;

    root.append(h("h2", { id: "places" }, "Send by hand to"), group, connected);
    if (picker) root.append(picker);

    // Tag rules
    if (s.dests.length) {
      const rules = h("div.group");
      s.rules.forEach((r, i) => {
        rules.append(
          h(
            "div.row",
            {},
            field(r.tag, "#tag", (v) => (r.tag = v.replace(/^#*/, "#"))),
            h("span", { style: "color:var(--faint)" }, "→"),
            select(
              r.dest,
              s.dests.map((d) => [d.id, d.name || "Untitled"]),
              (v) => (r.dest = v),
            ),
            h("div.label"),
            h(
              "button.ghost",
              {
                title: "Remove",
                "aria-label": "Remove",
                onclick: () => (s.rules.splice(i, 1), save(true)),
              },
              icon("x", 15),
            ),
          ),
        );
      });
      rules.append(
        h(
          "div.adders",
          {},
          h(
            "button.btn",
            {
              onclick: () => (
                s.rules.push({ tag: "#", dest: s.dests[0].id }),
                save(true)
              ),
            },
            icon("plus", 13),
            "Tag",
          ),
        ),
      );
      root.append(h("h2", {}, "Tags"), rules);
    }

    // Calls
    root.append(
      h("h2", {}, "Calls"),
      h(
        "div.group",
        {},
        row(
          "Offer notes when a call starts",
          sw(
            s.calls,
            (v) => ((s.calls = v), save()),
            "Offer notes when a call starts",
          ),
        ),
        ...s.quiet_apps.map((a) =>
          row(
            h("span", {}, "Never for ", h("b", {}, a)),
            h(
              "button.ghost",
              {
                title: "Ask again",
                "aria-label": `Ask again for ${a}`,
                onclick: () => (
                  (s.quiet_apps = s.quiet_apps.filter((x) => x !== a)),
                  save(true)
                ),
              },
              icon("x", 15),
            ),
          ),
        ),
      ),
    );

    // Agents
    if (state.agents.length)
      root.append(
        h("h2", {}, "Agents"),
        h(
          "div.group",
          {},
          row(
            `Teach ${state.agents.join(" and ")} where notes are`,
            sw(
              s.skill_auto,
              (v) => ((s.skill_auto = v), save()),
              "Teach agents",
            ),
          ),
        ),
      );

    // General
    root.append(
      h("h2", {}, "General"),
      h(
        "div.group",
        {},
        row(
          "Start with Windows",
          sw(auto, (v) => api.autostart(v), "Start with Windows"),
        ),
        row(
          "Name the source app",
          sw(
            s.keep_source,
            (v) => ((s.keep_source = v), save()),
            "Name the source app",
          ),
        ),
        row("Note size", noteSize()),
        row(
          "Appearance",
          select(
            s.theme,
            [
              ["system", "System"],
              ["light", "Light"],
              ["dark", "Dark"],
            ],
            (v) => (s.theme = v as Settings["theme"]),
          ),
        ),
        row(
          "Setup",
          h("button.btn", { onclick: () => hooks.setup() }, "Run again"),
        ),
      ),
      h("p.version", {}, "Tuck 0.1.0"),
    );
  };

  paint();
}
