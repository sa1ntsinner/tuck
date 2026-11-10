import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { api, errorText, type Note, type Settings, type Step } from "./api";
import { onboarding } from "./onboarding";
import { settingsPage } from "./settings";
import { ago, applyTheme, firstLine, h, icon, keyLabel, render, routeOf, toggleLine, when } from "./ui";

const win = getCurrentWindow();

type Tab = "deck" | "stickies" | "sent";

export const shared = { holdOpen: false };

export function mount(root: HTMLElement) {
  let tab: Tab = "deck";
  let query = "";
  let deck: Note[] = [];
  let sent: Note[] = [];
  let settings: Settings | null = null;
  let editing: string | null = null;
  let snackTimer = 0;
  let shownAt = performance.now();
  let sortReady = false;
  let inboxOk = true;
  const seen = new Set<string>();

  const search = h("input", { placeholder: "Search", spellcheck: false });
  const tabs = h("div.tabs");
  const list = h("div.list");
  const footer = h("footer");
  const shell = h(
    "div.deck",
    {},
    h(
      "header",
      {},
      h("h1", {}, "Tuck"),
      h("button.ghost", { title: "New note", "aria-label": "New note", onclick: () => api.newCapture("note") }, icon("plus", 17)),
      h("button.ghost", { title: "Settings", "aria-label": "Settings", onclick: () => openSettings() }, icon("gear", 17)),
    ),
    h("label.search", {}, icon("search", 14), search),
    tabs,
    list,
    footer,
  );
  root.append(shell);

  const visible = (): Note[] => {
    const pool = tab === "sent" ? sent : deck.filter((n) => (tab === "stickies" ? !!n.pin : !n.pin));
    const q = query.trim().toLowerCase();
    return q ? pool.filter((n) => `${n.text} ${n.app}`.toLowerCase().includes(q)) : pool;
  };

  const paintTabs = () => {
    const count = { deck: deck.filter((x) => !x.pin).length, stickies: deck.filter((x) => x.pin).length, sent: 0 };
    const names: [Tab, string][] = [
      ["deck", "Deck"],
      ["stickies", "On screen"],
      ["sent", "Sent"],
    ];
    tabs.replaceChildren(
      ...names.map(([t, label]) =>
        h(`button${t === tab ? ".on" : ""}`, {
          onclick: () => {
            tab = t;
            editing = null;
            paint();
          },
        }, label, count[t] ? h("span.n", {}, String(count[t])) : null),
      ),
    );
  };

  const row = (n: Note): HTMLElement => {
    const el = h("div.note");
    if (!seen.has(n.id)) {
      if (seen.size) el.classList.add("fresh");
      seen.add(n.id);
    }
    if (editing === n.id) {
      const ta = h("textarea", { value: n.text, rows: Math.min(12, n.text.split("\n").length + 1) });
      const done = async () => {
        editing = null;
        if (ta.value.trim() && ta.value !== n.text) await api.update({ ...n, text: ta.value.trim() });
        else paint();
      };
      ta.addEventListener("blur", done);
      ta.addEventListener("keydown", (e) => {
        if (e.key === "Escape" || (e.key === "Enter" && e.ctrlKey)) {
          e.preventDefault();
          e.stopPropagation();
          ta.blur();
        }
      });
      el.append(ta);
      requestAnimationFrame(() => {
        ta.focus();
        ta.setSelectionRange(ta.value.length, ta.value.length);
      });
    } else {
      const body = h("div.body", { html: render(n.text) });
      body.addEventListener("click", async (e) => {
        const t = e.target as HTMLElement;
        const box = t.closest<HTMLElement>(".box");
        if (box) return void api.update({ ...n, text: toggleLine(n.text, +box.dataset.line!) });
        if (t.closest("[data-link]") || tab === "sent") return;
        editing = n.id;
        paint();
      });
      el.append(body);
    }

    const meta = h("div.meta");
    const bits: string[] = [];
    bits.push(tab === "sent" && n.sent_at ? ago(n.sent_at) : ago(n.created));
    if (n.remind) bits.push(`remind ${when(n.remind)}`);
    if (n.pin) bits.push(n.pin.mode === "app" ? `on ${n.pin.app.replace(/\.exe$/i, "")}` : n.pin.mode === "desk" ? "on the desktop" : "floating");
    meta.append(h("span", {}, bits.join(" · ")));
    const d = tab === "deck" && settings ? routeOf(settings, n.text, n.to) : undefined;
    if (d) meta.append(h("span.to", {}, `→ ${d.name}`));
    if (tab === "sent" && n.sent) meta.append(h("span.to", {}, `→ ${n.sent}`));

    const acts = h("div.actions");
    const act = (name: string, label: string, fn: () => void) =>
      acts.append(h("button.ghost", { title: label, "aria-label": label, onclick: (e: Event) => (e.stopPropagation(), fn()) }, icon(name, 15)));
    if (tab === "sent") {
      act("undo", "Back to the deck", () => api.restore(n.id).then(() => snack("Back in the deck")));
    } else {
      act("send", d ? `Send to ${d.name}` : "Send", () => sendOne(n, el));
      act("pin", n.pin ? "Take off the screen" : "Put on the screen", () => api.pin(n.id, n.pin ? "" : "float"));
    }
    act("copy", "Copy", () => navigator.clipboard.writeText(n.text).then(() => snack("Copied")));
    act("trash", "Delete", () => leave(el).then(() => api.remove(n.id)).then(() => snack("Deleted", () => api.restore(n.id))));
    meta.append(acts);
    el.append(meta);
    return el;
  };

  const sendOne = async (n: Note, el: HTMLElement) => {
    if (!settings || !routeOf(settings, n.text, n.to)) return openSettings("places");
    try {
      await leave(el);
      snack(`Sent to ${await api.send(n.id)}`);
    } catch (e) {
      el.classList.remove("leaving");
      snack(errorText(e));
    }
  };

  const leave = async (el: HTMLElement) => {
    el.classList.add("leaving");
    await wait(150);
  };

  const paintFooter = () => {
    footer.replaceChildren();
    if (tab !== "deck") return;
    const waiting = deck.filter((n) => !n.pin);
    if (!waiting.length) return;
    const routable = settings ? waiting.filter((n) => routeOf(settings!, n.text, n.to)).length : 0;
    if (routable)
      footer.append(
        h(
          "button.btn.text",
          {
            onclick: async () => {
              const r = await api.sendAll();
              snack(r.errors.length ? r.errors[0] : `Sent ${r.sent}`);
            },
          },
          routable === waiting.length ? "Send all" : `Send ${routable}`,
        ),
      );
    footer.append(
      h("div.grow"),
      sortReady
        ? h("button.btn.primary", { onclick: () => openSort() }, icon("spark", 14), "Sort")
        : h("button.btn.primary", { onclick: () => openIntro() }, "Set up sorting"),
    );
  };

  const paint = () => {
    paintTabs();
    paintFooter();
    const items = visible();
    const keep = list.scrollTop;
    list.replaceChildren(...items.map(row));
    list.scrollTop = keep;
    if (!items.length) list.append(emptyState());
  };

  const emptyState = () => {
    const key = (s: string) => h("kbd", {}, keyLabel(s));
    if (query) return h("p.empty", {}, "Nothing matches.");
    if (tab === "stickies") return h("p.empty", {}, "Nothing on the screen.", h("br"), key(settings?.sticky_key ?? "Alt+W"), " writes a sticky.");
    if (tab === "sent") return h("p.empty", {}, "Nothing sent yet.");
    if (!inboxOk) return h("p.empty", {}, "The notes folder was moved or renamed.", h("br"), h("br"), h("button.btn", { onclick: () => openSettings("sorting") }, "Choose it again"));
    return h("p.empty", {}, "Nothing waiting.", h("br"), "Hold the middle button, or press ", key(settings?.capture_key ?? "Alt+Q"), ".");
  };

  const snack = (msg: string, undo?: () => void) => {
    shell.querySelector(".snack")?.remove();
    clearTimeout(snackTimer);
    const s = h("div.snack", {}, h("span", {}, msg));
    if (undo) s.append(h("button", { onclick: () => (s.remove(), undo()) }, "Undo"));
    shell.append(s);
    snackTimer = window.setTimeout(() => s.remove(), 4000);
  };

  const reload = async () => {
    let st;
    [deck, sent, settings, st] = await Promise.all([api.notes("deck"), api.notes("sent"), api.settings(), api.sortState()]);
    sortReady = st.ready && !!st.library;
    inboxOk = st.inbox_ok;
    if (settings) applyTheme(settings.theme);
    if (!editing) paint();
  };

  // ---------------------------------------------------------- pages

  const page = (title: string, ...parts: HTMLElement[]) => {
    shell.querySelector(".page")?.remove();
    const p = h("div.page");
    p.append(h("header", {}, h("button.ghost", { title: "Back", "aria-label": "Back", onclick: () => closePage(p) }, icon("back", 17)), h("h1", {}, title)), ...parts);
    shell.append(p);
    return p;
  };

  const closePage = async (p: HTMLElement) => {
    p.classList.add("leaving");
    await wait(160);
    p.remove();
    reload();
  };

  const openSettings = async (focus?: string) => {
    const s = await api.settings();
    const body = h("div.scroll");
    const p = page("Settings", body);
    settingsPage(body, s, {
      saved: (next) => {
        settings = next;
        applyTheme(next.theme);
      },
      dialog: (open) => (shared.holdOpen = open),
      close: () => closePage(p),
      setup: () => openIntro(),
    });
    if (focus) setTimeout(() => body.querySelector(`#${focus}`)?.scrollIntoView({ block: "start" }), 80);
  };

  /** Sort: the model plans where each note goes; the person sees it grouped by place, then applies. */
  const openSort = async () => {
    const body = h("div.scroll", {}, h("div.working"), h("p.empty", {}, "Reading your library…"));
    const accept = h("button.btn.primary", { disabled: true }, "Sort");
    const p = page("Sort", body, h("footer", {}, h("div.grow"), accept));
    let steps: Step[];
    try {
      steps = await api.sortPlan();
    } catch (e) {
      body.replaceChildren(h("p.empty", {}, errorText(e), h("br"), h("br"), h("button.btn", { onclick: () => openSettings("sorting") }, "Settings")));
      return;
    }
    if (!steps.length) return void body.replaceChildren(h("p.empty", {}, "Nothing to sort."));
    const on = new Set(steps.filter((x) => x.action !== "keep").map((x) => x.id));
    const groups = new Map<string, Step[]>();
    for (const st of steps) {
      const key = st.action === "keep" ? "" : st.path;
      groups.set(key, [...(groups.get(key) ?? []), st]);
    }
    const order = [...groups.keys()].sort((a, b) => (a === "" ? 1 : b === "" ? -1 : a.localeCompare(b)));
    const count = () => {
      accept.disabled = !on.size;
      accept.textContent = on.size ? `Sort ${on.size}` : "Sort";
    };
    body.replaceChildren(
      ...order.flatMap((key) => {
        const items = groups.get(key)!;
        const title = key ? key.replace(/\.md$/, "").split("/").slice(-2).join(" › ") : "Stays in the inbox";
        const rows = items.flatMap((st) => {
          const n = deck.find((d) => d.id === st.id);
          if (!n) return [];
          if (!key) return [h("div.proposal.kept", {}, h("div.what", {}, h("b", {}, firstLine(n))))];
          const check = h(`button.check${on.has(st.id) ? ".on" : ""}`, { "aria-label": "Include" }, icon("check", 12));
          check.addEventListener("click", () => {
            on.has(st.id) ? on.delete(st.id) : on.add(st.id);
            check.classList.toggle("on", on.has(st.id));
            count();
          });
          return [h("div.proposal", { title: st.text.trim() }, check, h("div.what", {}, h("b", {}, firstLine(n))))];
        });
        return [h("h2.place", { title: key }, title), h("div.places-group", {}, ...rows)];
      }),
    );
    count();
    accept.onclick = async () => {
      accept.disabled = true;
      accept.textContent = "Sorting…";
      const r = await api.sortApply(steps.filter((x) => on.has(x.id)));
      await closePage(p);
      snack(r.errors.length ? r.errors[0] : `Sorted ${r.done}`);
    };
  };

  const openIntro = () =>
    onboarding(shell, {
      hold: (open) => (shared.holdOpen = open),
      done: () => reload(),
    });

  // ---------------------------------------------------------- wiring

  search.addEventListener("input", () => {
    query = search.value;
    paint();
  });

  addEventListener("keydown", (e) => {
    const typing = (e.target as HTMLElement).closest("input, textarea, .keycap");
    if (e.key === "Escape") {
      const p = shell.querySelector<HTMLElement>(".page");
      if (typing === search && query) {
        search.value = query = "";
        paint();
      } else if (p && !typing) closePage(p);
      else if (!p && !typing) api.hide();
      return;
    }
    if (!typing && !e.ctrlKey && !e.altKey && e.key.length === 1 && !shell.querySelector(".page")) search.focus();
  });

  listen("notes", () => reload());
  listen("deck", async () => {
    shownAt = performance.now();
    shell.classList.remove("in");
    void shell.offsetWidth;
    shell.classList.add("in");
    await reload();
    if (settings && !settings.seen_intro) openIntro();
  });

  win.onFocusChanged(({ payload: focused }) => {
    if (focused || shared.holdOpen || performance.now() - shownAt < 600) return;
    setTimeout(() => {
      if (!document.hasFocus() && !shared.holdOpen) api.hide();
    }, 150);
  });

  reload().then(() => {
    if (settings && !settings.seen_intro) openIntro();
  });
}

function wait(ms: number) {
  return new Promise((r) => setTimeout(r, ms));
}
