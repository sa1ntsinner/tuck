import { api, errorText, type Provider, type Settings } from "./api";
import { h, icon } from "./ui";

const GROUPS: [Provider["kind"], string][] = [
  ["cli", "With your subscription"],
  ["key", "With an API key"],
  ["local", "On this computer"],
  ["custom", "Your own server"],
];

const GET: Record<string, string> = {
  "claude-code": "https://claude.com/claude-code",
  codex: "https://developers.openai.com/codex",
  "gemini-cli": "https://github.com/google-gemini/gemini-cli",
  ollama: "https://ollama.com/download",
  lmstudio: "https://lmstudio.ai",
};

const KEY_HINT: Record<string, string> = {
  anthropic: "sk-ant-…",
  openai: "sk-…",
  gemini: "AIza…",
  openrouter: "sk-or-…",
  groq: "gsk_…",
  xai: "xai-…",
};

/**
 * The list of everyone who can sort notes, and the few fields the chosen one
 * needs. Edits `s` in place and calls `save` after each change.
 */
export function providerPicker(s: Settings, save: () => Promise<void>, hold: (open: boolean) => void): HTMLElement {
  const root = h("div.picker");
  const status = h("p.test");
  let list: Provider[] = [];
  let models: string[] | null = null;
  let modelsFor = "";
  let typedKey = "";

  const loadModels = async () => {
    const id = s.ai_provider;
    const key = typedKey || (s.ai_key === "•" ? "•" : "");
    const tag = `${id}|${s.ai_url}|${key}`;
    if (tag === modelsFor) return;
    modelsFor = tag;
    models = null;
    try {
      models = await api.models(id, s.ai_url, key);
    } catch (e) {
      models = [];
      status.className = "test bad";
      status.textContent = errorText(e);
    }
    if (s.ai_provider === id) {
      const kind = list.find((x) => x.id === id)?.kind;
      if (!s.ai_model && models?.length && kind !== "cli") {
        s.ai_model = models[0];
        await save();
      }
      paint();
    }
  };

  const choose = async (p: Provider) => {
    if (s.ai_provider === p.id) return;
    s.ai_provider = p.id;
    s.ai_key = "";
    s.ai_model = "";
    s.ai_url = "";
    typedKey = "";
    models = null;
    modelsFor = "";
    status.textContent = "";
    await save();
    paint();
    if (p.ready || p.kind === "cli") loadModels();
  };

  const field = (value: string, placeholder: string, type: string, onchange: (v: string) => void) => {
    const f = h("input.field.wide", { value, placeholder, type, spellcheck: false });
    let t = 0;
    f.addEventListener("input", () => {
      clearTimeout(t);
      t = window.setTimeout(() => onchange(f.value.trim()), 400);
    });
    return f;
  };

  const modelControl = (p: Provider) => {
    if (models === null) return h("span.dim", {}, "Loading models…");
    if (!models.length && p.kind === "cli") return h("span.dim", {}, "Its default model");
    if (!models.length)
      return field(s.ai_model, "Model id", "text", async (v) => {
        s.ai_model = v;
        await save();
      });
    const opts = [...models];
    if (s.ai_model && !opts.includes(s.ai_model)) opts.unshift(s.ai_model);
    const sel = h("select.field.wide", {}, ...(p.kind === "cli" ? [h("option", { value: "", selected: !s.ai_model }, "Default")] : []), ...opts.map((m) => h("option", { value: m, selected: m === s.ai_model }, m)));
    sel.addEventListener("change", async () => {
      s.ai_model = sel.value;
      await save();
    });
    return sel;
  };

  const details = (p: Provider) => {
    const box = h("div.editor");
    const label = (text: string, control: Node) => h("label", {}, text, control);

    if (p.kind === "cli" || p.kind === "local") {
      if (!p.ready) {
        const get = GET[p.id];
        const missing = p.state === "Not installed" || p.state === "Not running";
        const again = h("button.btn", { onclick: () => refresh() }, "Check again");
        if (p.kind === "cli" && !missing) {
          box.append(h("div.acts", {}, h("button.btn.primary", { onclick: () => (hold(true), api.cliLogin(p.id)) }, "Sign in"), again));
        } else {
          box.append(
            h("div.acts", {}, get ? h("button.btn", { onclick: () => api.openUrl(get) }, p.state === "Not running" ? `Open ${p.name}` : `Get ${p.name}`) : null, again),
          );
        }
        return box;
      }
      box.append(label("Model", modelControl(p)));
    }

    if (p.kind === "key" || p.kind === "custom") {
      if (p.kind === "custom")
        box.append(
          label(
            "Server",
            field(s.ai_url, "https://…/v1", "url", async (v) => {
              s.ai_url = v;
              await save();
              loadModels();
            }),
          ),
        );
      box.append(
        label(
          "API key",
          field("", s.ai_key === "•" ? "Saved" : (KEY_HINT[p.id] ?? (p.kind === "custom" ? "Optional" : "Key")), "password", async (v) => {
            typedKey = v;
            s.ai_key = v || (s.ai_key === "•" ? "•" : "");
            await save();
            if (v) loadModels();
          }),
        ),
      );
      if (p.id === "openrouter") {
        const signIn = h("button.btn", {}, "Sign in with OpenRouter");
        signIn.addEventListener("click", async () => {
          signIn.textContent = "Waiting for the browser…";
          hold(true);
          try {
            await api.openrouterLogin();
            const fresh = await api.settings();
            s.ai_provider = fresh.ai_provider;
            s.ai_key = fresh.ai_key;
            typedKey = "";
            modelsFor = "";
            loadModels();
          } catch (e) {
            status.className = "test bad";
            status.textContent = errorText(e);
          } finally {
            hold(false);
            paint();
          }
        });
        box.append(h("div.acts", {}, signIn));
      }
      if (s.ai_key || p.kind === "custom") box.append(label("Model", modelControl(p)));
    }

    const test = h("button.btn", {}, "Test");
    test.addEventListener("click", async () => {
      test.disabled = true;
      status.className = "test";
      status.textContent = "Asking…";
      try {
        await save();
        await api.testAi();
        status.className = "test ok";
        status.textContent = "Works";
      } catch (e) {
        status.className = "test bad";
        status.textContent = errorText(e);
      } finally {
        test.disabled = false;
      }
    });
    box.append(h("div.acts", {}, test, status));
    return box;
  };

  const paint = () => {
    root.replaceChildren();
    if (!list.length) return void root.append(h("div.working"));
    for (const [kind, title] of GROUPS) {
      const group = h("div.group");
      for (const p of list.filter((x) => x.kind === kind)) {
        const on = s.ai_provider === p.id;
        group.append(
          h(
            `button.row.pick${on ? ".on" : ""}`,
            { onclick: () => choose(p), "aria-pressed": String(on) },
            h(`span.radio${on ? ".on" : ""}`),
            h("span.label", {}, p.name),
            p.state ? h(`span.state${p.ready ? ".ready" : ""}`, {}, p.ready ? icon("check", 13) : null, p.state) : null,
          ),
        );
        if (on) group.append(details(p));
      }
      root.append(h("h3", {}, title), group);
    }
  };

  const refresh = async () => {
    hold(false);
    list = await api.providers();
    paint();
    const cur = list.find((p) => p.id === s.ai_provider);
    if (cur && (cur.ready || cur.kind === "key" || cur.kind === "custom") && (s.ai_key || cur.kind !== "key")) {
      modelsFor = "";
      loadModels();
    }
  };

  paint();
  refresh();
  return root;
}
