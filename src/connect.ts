import { api, errorText, type AppKind, type Found } from "./api";
import { h, icon } from "./ui";

interface Hooks {
  browse: () => Promise<string | null>;
  hold: (open: boolean) => void;
  done: (name: string) => void;
}

const APPS: [AppKind, string, "here" | "online"][] = [
  ["obsidian", "Obsidian", "here"],
  ["logseq", "Logseq", "here"],
  ["joplin", "Joplin", "here"],
  ["anytype", "Anytype", "here"],
  ["folder", "Markdown folder", "here"],
  ["notion", "Notion", "online"],
  ["todoist", "Todoist", "online"],
  ["tana", "Tana", "online"],
];

const TOKEN_PAGE: Partial<Record<AppKind, string>> = {
  notion: "https://www.notion.so/profile/integrations",
  todoist: "https://app.todoist.com/app/settings/integrations/developer",
};

const TOKEN_HINT: Partial<Record<AppKind, string>> = {
  notion: "ntn_…",
  todoist: "Token from Settings → Integrations → Developer",
  tana: "Token from Settings → API tokens",
};

/** Every app Tuck can send to. Each connects in a click or two; found ones come first. */
export function connectList(hooks: Hooks): HTMLElement {
  const root = h("div.picker.connect");
  let found: Found[] | null = null;
  let open: AppKind | null = null;

  const status = h("p.test");
  const fail = (e: unknown) => {
    status.className = "test bad";
    status.textContent = errorText(e);
  };
  const wait = (text: string) => {
    status.className = "test";
    status.textContent = text;
  };

  const go = async (kind: AppKind, a = "", b = "") => {
    try {
      hooks.done(await api.connect(kind, a, b));
    } catch (e) {
      fail(e);
    }
  };

  const tokenField = (kind: AppKind, onToken: (t: string) => void) => {
    const f = h("input.field.wide", { type: "password", placeholder: TOKEN_HINT[kind] ?? "Token", spellcheck: false });
    let t = 0;
    f.addEventListener("input", () => {
      clearTimeout(t);
      t = window.setTimeout(() => f.value.trim() && onToken(f.value.trim()), 350);
    });
    requestAnimationFrame(() => f.focus());
    return f;
  };

  const details = (kind: AppKind): HTMLElement => {
    const box = h("div.editor");
    status.textContent = "";
    const here = (found ?? []).filter((f) => f.kind === kind);

    if (kind === "obsidian" || kind === "logseq" || kind === "folder") {
      const acts = h("div.acts");
      for (const f of here) acts.append(h("button.btn.primary", { title: f.path, onclick: () => go(kind, f.path) }, f.name));
      acts.append(
        h("button.btn", {
          onclick: async () => {
            const p = await hooks.browse();
            if (p) go(kind, p);
          },
        }, icon("folder", 14), here.length ? "Other" : "Choose folder"),
      );
      box.append(acts);
    }

    if (kind === "joplin") {
      const b = h("button.btn.primary", {}, "Connect");
      b.addEventListener("click", async () => {
        b.disabled = true;
        wait("Click Allow in Joplin");
        hooks.hold(true);
        await go("joplin");
        hooks.hold(false);
        b.disabled = false;
      });
      box.append(h("div.acts", {}, b));
    }

    if (kind === "anytype") {
      const b = h("button.btn.primary", {}, "Connect");
      b.addEventListener("click", async () => {
        let challenge: string;
        try {
          challenge = await api.anytypeChallenge();
        } catch (e) {
          return fail(e);
        }
        const code = h("input.field.code", { inputmode: "numeric", maxlength: 4, placeholder: "0000", "aria-label": "Code from Anytype" });
        code.addEventListener("input", () => {
          code.value = code.value.replace(/\D/g, "");
          if (code.value.length === 4) go("anytype", challenge, code.value);
        });
        b.replaceWith(h("label", {}, "Code Anytype shows", code));
        code.focus();
      });
      box.append(h("div.acts", {}, b));
    }

    if (kind === "notion" || kind === "todoist" || kind === "tana") {
      const page = TOKEN_PAGE[kind];
      const pages = h("div.acts");
      box.append(
        h("div.acts", {}, page ? h("button.btn", { onclick: () => api.openUrl(page) }, "Get a token") : null),
        tokenField(kind, async (token) => {
          if (kind !== "notion") return go(kind, token);
          wait("Looking for pages…");
          try {
            const list = await api.notionPages(token);
            status.textContent = list.length ? "" : "No pages yet. In Notion, open a page, ••• → Connections, add the integration.";
            pages.replaceChildren(...list.slice(0, 12).map((p) => h("button.btn", { onclick: () => go("notion", token, p.id) }, p.title)));
          } catch (e) {
            fail(e);
          }
        }),
        pages,
      );
    }

    box.append(status);
    return box;
  };

  const paint = () => {
    root.replaceChildren();
    for (const [where, title] of [["here", "On this computer"], ["online", "Online"]] as const) {
      const group = h("div.group");
      for (const [kind, name, w] of APPS.filter((x) => x[2] === where)) {
        const n = found?.filter((f) => f.kind === kind).length ?? 0;
        const on = open === kind;
        group.append(
          h(
            `button.row.pick${on ? ".on" : ""}`,
            { onclick: () => ((open = on ? null : kind), paint()), "aria-expanded": String(on) },
            h("span.label", {}, name),
            w === "here" && n ? h("span.state.ready", {}, icon("check", 13), "Found") : null,
          ),
        );
        if (on) group.append(details(kind));
      }
      root.append(h("h3", {}, title), group);
    }
  };

  paint();
  api.foundApps().then((f) => {
    found = f;
    // One obvious app on this PC opens by itself.
    const kinds = [...new Set(f.map((x) => x.kind))];
    if (!open && kinds.length === 1) open = kinds[0];
    paint();
  });
  return root;
}
