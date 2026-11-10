import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import { api, errorText } from "./api";
import { applyTheme, COLORS, h, icon, render, toggleLine } from "./ui";

const win = getCurrentWindow();

export async function mount(root: HTMLElement) {
  const id = new URLSearchParams(location.search).get("id") ?? "";
  let note = await api.note(id);
  if (!note) return win.close();
  let editing = Date.now() - note.created < 6000;
  let palette = false;
  let geomTimer = 0;
  let zoomTimer = 0;
  let zoom = note.pin?.zoom || 1;

  const settings = await api.settings().catch(() => null);
  if (settings) applyTheme(settings.theme);

  const card = h("div.sticky");
  const content = h("div.content");
  root.append(card);

  const persistGeom = () => {
    clearTimeout(geomTimer);
    geomTimer = window.setTimeout(async () => {
      const size = await win.innerSize();
      const scale = await win.scaleFactor();
      await api.stickyGeom(id, size.width / scale, size.height / scale, zoom, note!.color).catch(() => {});
    }, 450);
  };

  const save = async (text: string) => {
    if (!note || text === note.text) return;
    note = { ...note, text };
    await api.update(note).catch(() => {});
  };

  const paint = () => {
    if (!note) return;
    card.dataset.color = note.color || "butter";
    card.style.setProperty("--zoom", String(zoom));
    card.classList.toggle("editing", editing);
    const mode = note.pin?.mode ?? "float";
    const tool = (name: string, title: string, fn: () => void, on = false) =>
      h(`button.ghost${on ? ".on" : ""}`, { title, onclick: (e: Event) => (e.stopPropagation(), fn()) }, icon(name, 13));
    const appName = note.pin?.app?.replace(/\.exe$/i, "") ?? "";
    const grip = h(
      "div.grip",
      { "data-tauri-drag-region": true },
      h(
        "div.tools",
        {},
        h("button.ghost", { title: "Colour", "aria-label": "Colour", onclick: () => ((palette = !palette), paint()) }, h("span", { style: "width:10px;height:10px;border-radius:50%;background:currentColor;opacity:.6;display:block" })),
        mode === "app"
          ? tool("window", `Only on ${appName}`, () => api.pin(id, "float"))
          : mode === "desk"
            ? tool("layers", "On the desktop. Click to float on top", () => api.pin(id, "float"))
            : tool("float", "On top. Click to put on the desktop", () => api.pin(id, "desk")),
        tool("send", "Send", async () => {
          try {
            await api.send(id);
          } catch (e) {
            flash(errorText(e));
          }
        }),
        tool("x", "Back to the deck", async () => {
          card.classList.add("gone");
          await wait(160);
          await api.pin(id, "");
        }),
      ),
    );

    const swatches = palette
      ? h(
          "div.swatches",
          {},
          ...COLORS.map((c) =>
            h(`button${note!.color === c ? ".on" : ""}`, {
              title: c,
              style: `background:var(--c-${c})`,
              onclick: () => {
                note = { ...note!, color: c };
                palette = false;
                persistGeom();
                paint();
              },
            }),
          ),
        )
      : null;

    content.replaceChildren();
    if (editing) {
      const ta = h("textarea", { value: note.text, spellcheck: true, placeholder: "Write…" });
      ta.addEventListener("input", () => save(ta.value));
      ta.addEventListener("blur", () => {
        editing = false;
        save(ta.value.trim()).then(paint);
      });
      ta.addEventListener("keydown", (e) => {
        if (e.key === "Escape") ta.blur();
      });
      content.append(ta);
      requestAnimationFrame(() => {
        ta.focus();
        ta.setSelectionRange(ta.value.length, ta.value.length);
      });
    } else {
      const body = h("div.body", { html: render(note.text) });
      body.addEventListener("click", async (e) => {
        const box = (e.target as HTMLElement).closest<HTMLElement>(".box");
        if (box) {
          await save(toggleLine(note!.text, +box.dataset.line!));
          paint();
          return;
        }
        if ((e.target as HTMLElement).closest("[data-link]")) return;
        editing = true;
        paint();
      });
      content.append(body);
    }

    const resize = h("div.resize");
    resize.addEventListener("mousedown", (e) => {
      e.preventDefault();
      win.startResizeDragging("SouthEast");
    });
    card.replaceChildren(grip, ...(swatches ? [swatches] : []), content, resize);
  };

  const flash = (msg: string) => {
    card.querySelector(".flash")?.remove();
    const z = h("div.flash", {}, msg);
    card.append(z);
    setTimeout(() => z.remove(), 2400);
  };

  // Ctrl + wheel grows or shrinks the whole note.
  addEventListener(
    "wheel",
    async (e) => {
      if (!e.ctrlKey) return;
      e.preventDefault();
      const next = Math.min(2, Math.max(0.7, Math.round((zoom + (e.deltaY < 0 ? 0.1 : -0.1)) * 10) / 10));
      if (next === zoom) return;
      const size = await win.innerSize();
      const scale = await win.scaleFactor();
      const ratio = next / zoom;
      zoom = next;
      card.style.setProperty("--zoom", String(zoom));
      await win.setSize(new LogicalSize(Math.round((size.width / scale) * ratio), Math.round((size.height / scale) * ratio)));
      clearTimeout(zoomTimer);
      card.querySelector(".flash")?.remove();
      const z = h("div.flash", {}, `${Math.round(zoom * 100)}%`);
      card.append(z);
      zoomTimer = window.setTimeout(() => z.remove(), 700);
      persistGeom();
    },
    { passive: false },
  );

  win.onMoved(persistGeom);
  win.onResized(persistGeom);

  listen("notes", async () => {
    if (editing) return;
    const fresh = await api.note(id);
    if (!fresh || !fresh.pin) return;
    note = fresh;
    paint();
  });

  paint();
}

function wait(ms: number) {
  return new Promise((r) => setTimeout(r, ms));
}
