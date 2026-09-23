/**
 * Stack of the catalog: what the user picked to install, where and how.
 * Persisted in localStorage (per machine, never shared); the drawer on
 * `/llm/catalog` and the buttons on every card/detail read the same instance.
 *
 * Items are catalog ids, or inline canonical components (imported from a tool
 * with `agentkit_import_installed`) that have no id in the catalog.
 */
import type { Component, Policy, Scope } from "./catalog";

export interface StackItem {
  id: string;
  kind: string;
  name: string;
  category?: string;
  description?: string;
  /** Inline component (not in the catalog). */
  component?: Component | null;
}

interface Persisted {
  v: 1;
  name: string;
  items: StackItem[];
  targets: string[];
  targetsAuto: boolean;
  scope: Scope;
  projectDir: string | null;
  policy: Policy;
}

const KEY = "omniget.central.stack.v1";

function load(): Persisted | null {
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return null;
    const v = JSON.parse(raw) as Persisted;
    return v && v.v === 1 && Array.isArray(v.items) ? v : null;
  } catch {
    return null;
  }
}

class StackStore {
  name = $state("");
  items = $state<StackItem[]>([]);
  /** Chosen tool ids. With `targetsAuto`, the drawer refills them from detection. */
  targets = $state<string[]>([]);
  targetsAuto = $state(true);
  scope = $state<Scope>("project");
  projectDir = $state<string | null>(null);
  policy = $state<Policy>("rename");
  open = $state(false);
  /** Bumped when the drawer saves a collection, so the sidebar reloads. */
  rev = $state(0);

  constructor() {
    const p = typeof localStorage === "undefined" ? null : load();
    if (p) {
      this.name = p.name ?? "";
      this.items = p.items;
      this.targets = p.targets ?? [];
      this.targetsAuto = p.targetsAuto ?? true;
      this.scope = p.scope ?? "project";
      this.projectDir = p.projectDir ?? null;
      this.policy = p.policy ?? "rename";
    }
  }

  get count() {
    return this.items.length;
  }

  has(id: string): boolean {
    return this.items.some((i) => i.id === id);
  }

  add(item: StackItem) {
    if (this.has(item.id)) return;
    this.items = [...this.items, item];
    this.save();
  }

  addMany(list: StackItem[]) {
    const fresh = list.filter((i) => !this.has(i.id));
    if (!fresh.length) return 0;
    this.items = [...this.items, ...fresh];
    this.save();
    return fresh.length;
  }

  remove(id: string) {
    this.items = this.items.filter((i) => i.id !== id);
    this.save();
  }

  toggle(item: StackItem) {
    if (this.has(item.id)) this.remove(item.id);
    else this.add(item);
  }

  move(id: string, delta: number) {
    const i = this.items.findIndex((x) => x.id === id);
    const j = i + delta;
    if (i < 0 || j < 0 || j >= this.items.length) return;
    const next = [...this.items];
    [next[i], next[j]] = [next[j], next[i]];
    this.items = next;
    this.save();
  }

  clear() {
    this.items = [];
    this.name = "";
    this.save();
  }

  setTargets(ids: string[], auto = false) {
    this.targets = [...new Set(ids)];
    this.targetsAuto = auto;
    this.save();
  }

  toggleTarget(id: string) {
    this.setTargets(this.targets.includes(id) ? this.targets.filter((t) => t !== id) : [...this.targets, id]);
  }

  set<K extends "scope" | "projectDir" | "policy" | "name">(k: K, v: StackStore[K]) {
    (this as unknown as Record<string, unknown>)[k] = v;
    this.save();
  }

  catalogIds(): string[] {
    return this.items.filter((i) => !i.component).map((i) => i.id);
  }

  inlineComponents(): Component[] {
    return this.items.filter((i) => i.component).map((i) => i.component as Component);
  }

  save() {
    try {
      const p: Persisted = {
        v: 1,
        name: this.name,
        items: this.items,
        targets: this.targets,
        targetsAuto: this.targetsAuto,
        scope: this.scope,
        projectDir: this.projectDir,
        policy: this.policy,
      };
      localStorage.setItem(KEY, JSON.stringify(p));
    } catch {
      /* storage off or full: the stack lives only in memory */
    }
  }
}

export const stack = new StackStore();

/**
 * Guard reports of catalog items scanned in this session (detail page, plan
 * dialog), so the cards can show the real seal instead of "not scanned".
 */
export const guardSeals: Record<string, import("./guard").GuardReport> = $state({});
