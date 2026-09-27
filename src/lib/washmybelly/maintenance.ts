/// Maintenance tasks from `maintenance.list` and the pure logic of the
/// screen: order, the duration label, what the card warns about.

export type MaintenanceTask = {
  id: string;
  title: string;
  what: string;
  admin: boolean;
  minutes: number;
  long: boolean;
  group: "network" | "apps" | "search" | "disk" | "system" | string;
  available: boolean;
  reason?: string | null;
  command: string;
  undo: "none";
};

export type MaintenanceList = { os: "macos" | "windows" | "linux"; tasks: MaintenanceTask[] };

export const GROUP_ORDER = ["network", "apps", "search", "disk", "system"];

/// Available tasks first, then by group, quick ones before long ones.
export function sortTasks(tasks: MaintenanceTask[]): MaintenanceTask[] {
  const g = (t: MaintenanceTask) => {
    const i = GROUP_ORDER.indexOf(t.group);
    return i < 0 ? GROUP_ORDER.length : i;
  };
  return [...tasks].sort(
    (a, b) => Number(b.available) - Number(a.available) || g(a) - g(b) || Number(a.long) - Number(b.long) || a.minutes - b.minutes,
  );
}

/// `{ key, count }` for the duration tag ("about a minute", "about 20 min").
export function durationLabel(minutes: number): { key: string; count: number } {
  if (minutes <= 1) return { key: "washmybelly.maintenance.duration.quick", count: 1 };
  if (minutes >= 60) return { key: "washmybelly.maintenance.duration.hours", count: Math.round(minutes / 60) };
  return { key: "washmybelly.maintenance.duration.minutes", count: minutes };
}

/// The plan item id of a task (what `wmb_plan` knows it by).
export function planItemId(taskId: string): string {
  return `maintenance:${taskId}`;
}

/// i18n key of a task's title or text; falls back to the English the core sent.
export function taskKey(taskId: string, field: "title" | "what"): string {
  return `washmybelly.maintenance.task.${taskId}.${field}`;
}
