import { invoke } from "@tauri-apps/api/core";
import { applyTheme } from "./ui";

const report = (msg: string) => invoke("log", { msg }).catch(() => {});
addEventListener("error", (e) => report(`${e.message} @ ${e.filename}:${e.lineno}`));
addEventListener("unhandledrejection", (e) => report(`unhandled: ${e.reason instanceof Error ? e.reason.stack : String(e.reason)}`));

const view = new URLSearchParams(location.search).get("w") ?? "deck";
document.documentElement.dataset.view = view;
applyTheme();

const views: Record<string, () => Promise<{ mount(root: HTMLElement): void }>> = {
  capture: () => import("./capture"),
  deck: () => import("./deck"),
  sticky: () => import("./sticky"),
  toast: () => import("./toast"),
};

(views[view] ?? views.deck)().then((m) => m.mount(document.getElementById("root")!));

// No browser context menu or reload keys inside an app window.
addEventListener("contextmenu", (e) => {
  if (!(e.target as HTMLElement).closest("textarea, input")) e.preventDefault();
});
addEventListener("keydown", (e) => {
  if (e.key === "F5" || (e.ctrlKey && e.key.toLowerCase() === "r")) e.preventDefault();
});
