import { open } from "@tauri-apps/plugin-dialog";
import { api, errorText, type Settings } from "./api";
import { providerPicker } from "./providers";
import { h, icon, keyLabel } from "./ui";

interface Hooks {
  hold: (open: boolean) => void;
  done: () => void;
}

const last = (p: string, n = 2) => p.split(/[\\/]/).filter(Boolean).slice(-n).join(" › ");

/** First run: what the keys do, where notes wait, who sorts them, done. */
export async function onboarding(shell: HTMLElement, hooks: Hooks) {
  let s: Settings = await api.settings();
  let step = 0;
  const vaults = await api.vaults();

  const save = async () => {
    await api.setSettings(s);
    const fresh = await api.settings();
    s.ai_key = fresh.ai_key;
    if (step === 2) next.textContent = (await api.sortState()).ready ? "Continue" : "Skip";
  };

  const browse = async () => {
    hooks.hold(true);
    try {
      const picked = await open({ directory: true });
      return typeof picked === "string" ? picked : null;
    } finally {
      hooks.hold(false);
    }
  };

  const page = h("div.page.onboard");
  const bar = h("div.progress");
  const body = h("div.scroll");
  const back = h("button.btn.text", {}, "Back");
  const next = h("button.btn.primary");
  page.append(bar, body, h("footer", {}, back, h("div.grow"), next));
  shell.querySelector(".page")?.remove();
  shell.append(page);

  back.addEventListener("click", () => go(step - 1));

  const go = (to: number) => {
    step = Math.max(0, Math.min(3, to));
    paint();
  };

  // ------------------------------------------------------------ steps

  const keys = () => {
    const k = (combo: string) => h("span.k", {}, ...keyLabel(combo).split(" + ").map((x) => h("kbd", {}, x)));
    return [
      h("h1", {}, "Catch it", h("br"), "before it's gone."),
      h(
        "div.keys",
        {},
        h("span.k", {}, h("kbd", {}, "Hold middle button")),
        h("span", {}, "New note, anywhere"),
        k(s.capture_key),
        h("span", {}, "New note"),
        k(s.sticky_key),
        h("span", {}, "Sticky on the screen"),
        k(s.deck_key),
        h("span", {}, "All notes"),
      ),
    ];
  };

  /** Inbox candidates: the current one, each vault's inbox, this PC. */
  const places = () => {
    const out: { path: string; label: string }[] = [];
    const add = (path: string, label: string) => {
      if (!out.some((o) => o.path.toLowerCase() === path.toLowerCase())) out.push({ path, label });
    };
    if (s.inbox) add(s.inbox, last(s.inbox));
    for (const v of vaults.slice(0, 3)) add(`${v.path}\\Inbox`, `${v.name} › Inbox`);
    add("", "This PC only");
    return out;
  };

  const where = () => {
    const group = h("div.group");
    for (const p of places()) {
      const on = s.inbox === p.path;
      group.append(
        h(
          `button.row.pick${on ? ".on" : ""}`,
          {
            onclick: async () => {
              s.inbox = p.path;
              if (p.path) s.library = "";
              await commit();
            },
          },
          h(`span.radio${on ? ".on" : ""}`),
          h("span.label", {}, p.label),
        ),
      );
    }
    group.append(
      h(
        "button.row.pick",
        {
          onclick: async () => {
            const p = await browse();
            if (!p) return;
            s.inbox = p;
            s.library = "";
            await commit();
          },
        },
        h("span.plus", {}, icon("folder", 15)),
        h("span.label", {}, "Choose a folder"),
      ),
    );

    const lib = h("div.group");
    const sortInto = async () => {
      const p = await browse();
      if (!p) return;
      s.library = p;
      await commit();
    };
    lib.append(
      h(
        "div.row",
        {},
        h("span.label", {}, state?.library ? last(state.library, 1) : "Not chosen"),
        h("button.btn", { onclick: sortInto }, state?.library ? "Change" : "Choose"),
      ),
    );
    return [h("h1", {}, "Where should", h("br"), "notes wait?"), group, h("p.path", {}, state?.inbox ?? ""), h("h3", {}, "Then sorted into"), lib, error];
  };

  const who = () => {
    const auto = h(
      "select.field",
      {},
      ...([
        ["off", "When I press Sort"],
        ["hourly", "Every hour"],
        ["evening", "Every evening"],
      ] as const).map(([v, l]) => h("option", { value: v, selected: s.sort_auto === v }, l)),
    );
    auto.addEventListener("change", async () => {
      s.sort_auto = auto.value as Settings["sort_auto"];
      await save();
    });
    return [
      h("h1", {}, "Who sorts", h("br"), "your notes?"),
      providerPicker(s, save, hooks.hold),
      h("h3", {}, "Sort"),
      h("div.group", {}, h("div.row", {}, h("span.label", {}, "Runs"), auto)),
    ];
  };

  const ready = async () => {
    const sw = (on: boolean, flip: (v: boolean) => void) => {
      const b = h(`button.switch${on ? ".on" : ""}`, { role: "switch", "aria-checked": String(on) });
      b.addEventListener("click", () => {
        on = !on;
        b.classList.toggle("on", on);
        b.setAttribute("aria-checked", String(on));
        flip(on);
      });
      return b;
    };
    const autoOn = await api.autostart();
    const agents = state?.agents ?? [];
    return [
      h("h1", {}, "Ready."),
      h(
        "div.group",
        {},
        h("div.row", {}, h("span.label", {}, "Start with Windows"), sw(autoOn, (v) => api.autostart(v))),
        agents.length
          ? h(
              "div.row",
              {},
              h("span.label", {}, `Teach ${agents.join(" and ")} where notes are`),
              sw(s.skill_auto, async (v) => {
                s.skill_auto = v;
                await save();
              }),
            )
          : null,
      ),
    ];
  };

  // ------------------------------------------------------------ paint

  let state: Awaited<ReturnType<typeof api.sortState>> | null = null;
  const error = h("p.status");

  const commit = async () => {
    error.textContent = "";
    try {
      await save();
    } catch (e) {
      error.textContent = errorText(e);
    }
    await paint();
  };

  const paint = async () => {
    state = await api.sortState();
    bar.replaceChildren(...[0, 1, 2, 3].map((i) => h(`i${i <= step ? ".on" : ""}`)));
    const parts = step === 0 ? keys() : step === 1 ? where() : step === 2 ? who() : await ready();
    const wrap = h(`div.step.s${step}`, {}, ...parts);
    body.replaceChildren(wrap);
    back.style.visibility = step ? "visible" : "hidden";
    next.textContent = step === 3 ? "Start" : step === 2 && !state.ready ? "Skip" : "Continue";
    next.onclick = async () => {
      if (step < 3) return go(step + 1);
      s.seen_intro = true;
      await save();
      page.classList.add("leaving");
      setTimeout(() => {
        page.remove();
        hooks.done();
      }, 160);
    };
  };

  paint();
}
