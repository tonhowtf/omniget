export type RoadmapTicket = {
  key: string;
  title: string;
  status: string;
  type: string;
  priority: string;
  area: string;
  created: string;
  updated: string;
  url: string;
  issue: string | null;
};

export type Roadmap = {
  updated: string;
  next_version: string;
  board: string;
  columns: string[];
  items: RoadmapTicket[];
};

export type RoadmapColumn = { id: string; items: RoadmapTicket[] };

const ORDER = ["progress", "todo", "backlog", "done", "wontfix"];

export function roadmapColumns(roadmap: Roadmap): RoadmapColumn[] {
  const known = ORDER.filter((id) => roadmap.columns.includes(id));
  const extra = roadmap.columns.filter((id) => !ORDER.includes(id));
  return [...known, ...extra]
    .map((id) => ({
      id,
      items: roadmap.items.filter((t) => t.status === id).sort((a, b) => b.updated.localeCompare(a.updated)),
    }))
    .filter((c) => c.items.length > 0);
}

export function roadmapErrorText(e: unknown): string {
  const raw = e instanceof Error ? e.message : String(e);
  try {
    const parsed = JSON.parse(raw) as { message?: unknown };
    if (typeof parsed.message === "string") return parsed.message;
  } catch {
    return raw;
  }
  return raw;
}
