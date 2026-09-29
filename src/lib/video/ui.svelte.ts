// In-page navigation for /video (a store, not sub-routes, like /music).

export type View =
  | { name: "home" }
  | { name: "movies" }
  | { name: "shows" }
  | { name: "show"; show: string }
  | { name: "courses" }
  | { name: "course"; id: number }
  | { name: "folders"; path?: string | null }
  | { name: "history" }
  | { name: "live"; group?: string | null }
  | { name: "anime"; id?: number | null }
  | { name: "sources" }
  | { name: "jellyfin"; source: number; parent?: string | null; title?: string | null }
  | { name: "addons"; addon?: number | null; type?: string | null; id?: string | null }
  | { name: "study" }
  | { name: "search"; q: string };

export const ui = $state<{ view: View; stack: View[]; search: string }>({
  view: { name: "home" },
  stack: [],
  search: "",
});

export function go(view: View) {
  ui.stack.push(ui.view);
  if (ui.stack.length > 50) ui.stack.shift();
  ui.view = view;
}

/** Top-level sections reset the back stack. */
export function goRoot(view: View) {
  ui.stack = [];
  ui.view = view;
}

export function back() {
  const prev = ui.stack.pop();
  if (prev) ui.view = prev;
}
