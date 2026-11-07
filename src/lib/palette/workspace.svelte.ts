import { parseColor, type Oklch } from "./color";
import { makeProject, newId, readProjectsFile, type PaletteDef, type PaletteNamer, type Project, type ProjectsFile } from "./project";
import type { Stop } from "./gradient";
import * as api from "./api";
import { serialRunner } from "./serial";

export type Tab = "color" | "scales" | "contrast" | "gradients" | "image" | "export";
export const TABS: Tab[] = ["color", "scales", "contrast", "gradients", "image", "export"];

type Phase = "loading" | "ready" | "error";
type SaveState = "idle" | "saving" | "saved" | "error";

type Removed =
  | { kind: "project"; item: Project; index: number }
  | { kind: "palette"; item: PaletteDef; index: number; projectId: string }
  | { kind: "stop"; item: Stop; index: number; projectId: string };

export type Toast = { id: number; text: string; undo: boolean; tone: "info" | "success" | "error" };

const SAVE_DELAY = 450;
const UNDO_MS = 8000;

class Workspace {
  phase = $state<Phase>("loading");
  loadError = $state<string | null>(null);
  movedAside = $state<string | null>(null);
  file = $state<ProjectsFile>({ version: 1, activeId: null, projects: [] });
  saveState = $state<SaveState>("idle");
  saveError = $state<string | null>(null);
  tab = $state<Tab>("color");
  current = $state<Oklch>(parseColor("#3b82f6")!);
  toast = $state<Toast | null>(null);
  removed: Removed | null = null;
  private saveTimer: ReturnType<typeof setTimeout> | null = null;
  private toastTimer: ReturnType<typeof setTimeout> | null = null;
  private lastSaved = "";
  private runSave = serialRunner(() => this.saveOnce());
  private toastSeq = 0;

  project = $derived(this.file.projects.find((p) => p.id === this.file.activeId) ?? null);

  async load(fallbackName: string, untitled: PaletteNamer) {
    this.phase = "loading";
    this.loadError = null;
    try {
      const out = await api.loadProjects();
      this.file = readProjectsFile(out.data, fallbackName, untitled);
      this.movedAside = out.moved_aside;
      this.lastSaved = JSON.stringify(this.file);
      this.phase = "ready";
    } catch (e) {
      this.loadError = api.errorText(e);
      this.phase = "error";
    }
  }

  touch() {
    const p = this.project;
    if (p) p.updatedAt = Date.now();
    this.schedule();
  }

  schedule() {
    if (this.phase !== "ready") return;
    if (this.saveTimer) clearTimeout(this.saveTimer);
    this.saveTimer = setTimeout(() => void this.flush(), SAVE_DELAY);
  }

  flush(): Promise<void> {
    if (this.saveTimer) {
      clearTimeout(this.saveTimer);
      this.saveTimer = null;
    }
    return this.runSave();
  }

  private async saveOnce() {
    if (this.phase !== "ready") return;
    const snapshot = $state.snapshot(this.file);
    const text = JSON.stringify(snapshot);
    if (text === this.lastSaved) return;
    this.saveState = "saving";
    try {
      await api.saveProjects(snapshot);
      this.lastSaved = text;
      this.saveState = "saved";
      this.saveError = null;
    } catch (e) {
      this.saveState = "error";
      this.saveError = api.errorText(e);
    }
  }

  select(id: string) {
    this.file.activeId = id;
    this.schedule();
  }

  addProject(project: Project) {
    this.file.projects = [...this.file.projects, project];
    this.file.activeId = project.id;
    this.schedule();
  }

  createProject(name: string, nameOf: (key: string) => string) {
    this.addProject(makeProject(name, nameOf));
  }

  duplicate(copyName: string) {
    const p = this.project;
    if (!p) return;
    const copy: Project = JSON.parse(JSON.stringify($state.snapshot(p)));
    copy.id = newId("prj");
    copy.name = copyName;
    copy.palettes = copy.palettes.map((pal) => ({ ...pal, id: newId("pal") }));
    copy.updatedAt = Date.now();
    this.addProject(copy);
  }

  showToast(text: string, tone: Toast["tone"] = "info", undo = false) {
    if (this.toastTimer) clearTimeout(this.toastTimer);
    this.toast = { id: ++this.toastSeq, text, undo, tone };
    this.toastTimer = setTimeout(() => {
      this.toast = null;
      if (undo) this.removed = null;
    }, undo ? UNDO_MS : 3200);
  }

  dismissToast() {
    if (this.toastTimer) clearTimeout(this.toastTimer);
    this.toast = null;
  }

  deleteProject(message: (name: string) => string) {
    const index = this.file.projects.findIndex((p) => p.id === this.file.activeId);
    if (index < 0) return;
    const item = $state.snapshot(this.file.projects[index]) as Project;
    this.file.projects = this.file.projects.filter((_, i) => i !== index);
    this.file.activeId = this.file.projects[Math.min(index, this.file.projects.length - 1)]?.id ?? null;
    this.removed = { kind: "project", item, index };
    this.schedule();
    this.showToast(message(item.name), "info", true);
  }

  deletePalette(id: string, message: (name: string) => string) {
    const p = this.project;
    if (!p) return;
    const index = p.palettes.findIndex((x) => x.id === id);
    if (index < 0) return;
    const item = $state.snapshot(p.palettes[index]) as PaletteDef;
    p.palettes = p.palettes.filter((_, i) => i !== index);
    this.removed = { kind: "palette", item, index, projectId: p.id };
    this.touch();
    this.showToast(message(item.name), "info", true);
  }

  deleteStop(index: number, message: string) {
    const p = this.project;
    if (!p || p.gradient.stops.length <= 2) return;
    const item = $state.snapshot(p.gradient.stops[index]) as Stop;
    p.gradient.stops = p.gradient.stops.filter((_, i) => i !== index);
    this.removed = { kind: "stop", item, index, projectId: p.id };
    this.touch();
    this.showToast(message, "info", true);
  }

  undo() {
    const r = this.removed;
    if (!r) return;
    this.removed = null;
    this.dismissToast();
    if (r.kind === "project") {
      const list = [...this.file.projects];
      list.splice(Math.min(r.index, list.length), 0, r.item);
      this.file.projects = list;
      this.file.activeId = r.item.id;
      this.schedule();
      return;
    }
    const p = this.file.projects.find((x) => x.id === r.projectId);
    if (!p) return;
    this.file.activeId = p.id;
    if (r.kind === "palette") {
      const list = [...p.palettes];
      list.splice(Math.min(r.index, list.length), 0, r.item);
      p.palettes = list;
    } else {
      const list = [...p.gradient.stops];
      list.splice(Math.min(r.index, list.length), 0, r.item);
      p.gradient.stops = list;
    }
    this.touch();
  }
}

export const workspace = new Workspace();
