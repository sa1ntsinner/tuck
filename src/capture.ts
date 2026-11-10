import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import { api, errorText, type Ctx, type Settings } from "./api";
import { applyTheme, COLORS, h, parseRemind, routeOf } from "./ui";

const win = getCurrentWindow();

export function mount(root: HTMLElement) {
  let ctx: Ctx = { mode: "note", app: "", exe: "", title: "", clip: "", call: "", by_mouse: false };
  let settings: Settings | null = null;
  let busy = false;
  let error = "";
  let clipUsed = false;
  let openedAt = 0;
  let showKeys = false;
  let color: string = COLORS[0];
  let resizing = false;
  let resized = 0;

  const text = h("textarea", { spellcheck: true, rows: 2 });
  const line = h("div.line");
  const grip = h("div.resize");
  const card = h("div.capture", {}, text, line, grip);
  root.append(card);

  text.value = localStorage.getItem("tuck-draft") ?? "";

  // The window opens at the size set in Settings and only grows past it when the text needs room.
  const fit = () => {
    text.style.height = "auto";
    text.style.height = `${Math.min(text.scrollHeight, 480)}px`;
    if (resizing) return;
    const natural = text.offsetHeight + line.offsetHeight + 40;
    const w = settings?.note_w ?? 640;
    const hgt = Math.max(settings?.note_h ?? 170, natural);
    if (Math.abs(innerWidth - w) > 1 || Math.abs(innerHeight - hgt) > 1) win.setSize(new LogicalSize(w, hgt)).catch(() => {});
  };

  // Dragging the corner sets the new default size.
  grip.addEventListener("mousedown", (e) => {
    e.preventDefault();
    resizing = true;
    win.startResizeDragging("SouthEast");
  });
  win.onResized(() => {
    if (!resizing) return;
    clearTimeout(resized);
    resized = window.setTimeout(async () => {
      resizing = false;
      if (!settings) return;
      settings.note_w = Math.round(innerWidth);
      settings.note_h = Math.round(innerHeight);
      await api.setSettings(settings).catch(() => {});
      fit();
    }, 450);
  });

  const paint = () => {
    const sticky = ctx.mode === "sticky";
    card.dataset.color = color;
    text.placeholder = sticky ? "Write on the sticky" : "Catch the thought";
    line.replaceChildren();

    if (error) {
      line.append(h("span.error", {}, error));
    } else if (showKeys) {
      const k = (...keys: string[]) => keys.map((x) => h("kbd", {}, x));
      line.append(
        h(
          "div.keys",
          {},
          h("span", {}, ...k("Enter"), sticky ? "stick" : "tuck"),
          h("span", {}, ...k("Ctrl", "Enter"), "send"),
          h("span", {}, ...k("Ctrl", "P"), sticky ? "deck" : "stick"),
          ctx.app ? h("span", {}, ...k("Ctrl", "Shift", "P"), `on ${ctx.app}`) : null,
        ),
      );
    } else {
      const value = text.value;
      const r = parseRemind(value);
      if (r) line.append(h("span", {}, `Remind ${r.label}`));
      if (ctx.clip && !clipUsed && !value.includes(ctx.clip.slice(0, 40))) {
        line.append(h("button", { title: "Ctrl+Shift+V", onclick: addClip }, "Add clipboard"));
      }
      const d = settings && value.trim() ? routeOf(settings, value) : undefined;
      line.append(h("span.push.route", {}, d ? `→ ${d.name}` : ""));
    }
    fit();
  };

  const addClip = () => {
    const sep = text.value && !text.value.endsWith("\n") ? "\n" : "";
    text.value += sep + ctx.clip;
    clipUsed = true;
    text.focus();
    text.setSelectionRange(text.value.length, text.value.length);
    persist();
    paint();
  };

  const persist = () => {
    try {
      localStorage.setItem("tuck-draft", text.value);
    } catch {}
  };

  const close = async () => {
    card.classList.remove("in");
    card.classList.add("away");
    await wait(120);
    await api.dismiss();
    card.classList.remove("away");
  };

  const commit = async (action: "tuck" | "send" | "pin" | "pin_app") => {
    const value = text.value.trim();
    if (!value || busy) return;
    busy = true;
    error = "";
    try {
      const remind = parseRemind(value);
      card.classList.remove("in");
      card.classList.add("out");
      await api.save(
        { text: value, app: ctx.app, title: ctx.title, remind: remind?.at ?? null, color },
        action,
      );
      text.value = "";
      persist();
    } catch (e) {
      card.classList.remove("out");
      error = errorText(e);
      paint();
    } finally {
      busy = false;
    }
  };

  text.addEventListener("input", () => {
    error = "";
    persist();
    paint();
  });

  addEventListener("keyup", (e) => {
    if (e.key === "Control" && showKeys) {
      showKeys = false;
      paint();
    }
  });

  text.addEventListener("keydown", (e) => {
    const key = e.key.toLowerCase();
    if (e.key === "Control" && !e.repeat) {
      // Holding Ctrl alone shows what the keys do.
      setTimeout(() => {
        if (!showKeys && keyHeld) {
          showKeys = true;
          paint();
        }
      }, 450);
      keyHeld = true;
      return;
    }
    keyHeld = false;
    if (e.key === "Escape") {
      e.preventDefault();
      close();
    } else if (e.key === "Enter" && e.ctrlKey) {
      e.preventDefault();
      commit("send");
    } else if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      commit(ctx.mode === "sticky" ? "pin" : "tuck");
    } else if (key === "p" && e.ctrlKey && e.shiftKey) {
      e.preventDefault();
      if (ctx.app) commit("pin_app");
    } else if (key === "p" && e.ctrlKey) {
      e.preventDefault();
      commit(ctx.mode === "sticky" ? "tuck" : "pin");
    } else if (key === "v" && e.ctrlKey && e.shiftKey && ctx.clip) {
      e.preventDefault();
      addClip();
    } else if (e.key === "Enter" && e.shiftKey) {
      // Lists continue the way they do in a text editor.
      const before = text.value.slice(0, text.selectionStart);
      const current = before.slice(before.lastIndexOf("\n") + 1);
      const m = current.match(/^(\s*)(- \[[ xX]\] |- |\* |\d+\. )/);
      if (m) {
        e.preventDefault();
        if (current.trim() === m[2].trim()) {
          text.setRangeText("", text.selectionStart - current.length, text.selectionStart, "end");
        } else {
          const marker = m[2].startsWith("- [") ? "- [ ] " : /^\d+\./.test(m[2]) ? `${parseInt(m[2]) + 1}. ` : m[2];
          text.setRangeText(`\n${m[1]}${marker}`, text.selectionStart, text.selectionEnd, "end");
        }
        persist();
        paint();
      }
    }
  });
  let keyHeld = false;

  listen<Ctx>("capture", async (ev) => {
    ctx = ev.payload;
    openedAt = performance.now();
    clipUsed = false;
    resizing = false;
    // A fresh sheet each time, never the same colour twice in a row.
    const others = COLORS.filter((c) => c !== color);
    color = others[Math.floor(Math.random() * others.length)];
    error = "";
    showKeys = false;
    card.classList.remove("out", "away", "in");
    void card.offsetWidth;
    card.classList.add("in");
    paint();
    text.focus();
    text.setSelectionRange(text.value.length, text.value.length);
    settings = await api.settings().catch(() => null);
    if (settings) applyTheme(settings.theme);
    paint();
  });

  // Windows sends a stray blur while it hands focus over; only a blur that lasts counts.
  win.onFocusChanged(({ payload: focused }) => {
    if (focused || busy || performance.now() - openedAt < 600) return;
    setTimeout(() => {
      if (!document.hasFocus() && !busy) {
        persist();
        api.hide();
      }
    }, 150);
  });

  api.ctx().then((c) => {
    ctx = c;
    paint();
  });
}

function wait(ms: number) {
  return new Promise((r) => setTimeout(r, ms));
}
