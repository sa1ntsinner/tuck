import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { api } from "./api";
import { h, icon } from "./ui";

interface Toast {
  kind: "call" | "remind" | "sorted";
  title: string;
  body: string;
  id: string;
}

export function mount(root: HTMLElement) {
  let timer = 0;

  const act = async (el: HTMLElement, t: Toast, action: string) => {
    clearTimeout(timer);
    el.classList.add("out");
    await new Promise((r) => setTimeout(r, 170));
    await api.toastAction(t.kind, action, t.id);
  };

  const show = (t: Toast) => {
    clearTimeout(timer);
    const el = h("div.toast");
    const buttons =
      t.kind === "call"
        ? [
            h("button.btn.primary", { onclick: () => act(el, t, "notes") }, "Take notes"),
            h("button.btn", { onclick: () => act(el, t, "dismiss") }, "Not now"),
            h("button.btn.text", { onclick: () => act(el, t, "quiet") }, `Never for ${t.id}`),
          ]
        : t.kind === "sorted"
          ? [h("button.btn.primary", { onclick: () => act(el, t, "open") }, "Open")]
          : [
              h("button.btn.primary", { onclick: () => act(el, t, "open") }, "Open"),
              h("button.btn", { onclick: () => act(el, t, "snooze") }, "In 10 min"),
            ];
    el.append(
      h("b", {}, t.body),
      h("div.acts", {}, ...buttons),
      h("button.ghost.close", { title: "Close", "aria-label": "Close", onclick: () => act(el, t, "dismiss") }, icon("x", 14)),
    );
    root.replaceChildren(el);
    timer = window.setTimeout(() => act(el, t, "dismiss"), t.kind === "call" ? 16000 : 30000);
  };

  listen<Toast>("toast", ({ payload }) => show(payload));
  invoke<Toast | null>("pending_toast").then((t) => t && show(t));
}
