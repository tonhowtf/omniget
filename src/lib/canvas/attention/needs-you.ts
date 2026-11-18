export interface InboxItem {
  nodeId: string;
  name?: string;
  workspaceId: string;
  floorId?: string;
  reason: string;
  sinceMs?: number;
}

const REASONS = ["permission", "blocked", "stuck", "trust", "chrome_prompt", "login", "review"] as const;
export type Reason = (typeof REASONS)[number] | "other";

export const MAX_ROWS = 5;

export function reasonKey(reason: string): Reason {
  return (REASONS as readonly string[]).includes(reason) ? (reason as Reason) : "other";
}

export function isUrgent(reason: string): boolean {
  return reason !== "review";
}

export function orderInbox(items: InboxItem[]): InboxItem[] {
  const seen = new Set<string>();
  return items
    .filter((i) => i && i.nodeId && i.workspaceId)
    .sort(
      (a, b) =>
        Number(isUrgent(b.reason)) - Number(isUrgent(a.reason)) ||
        (a.sinceMs ?? 0) - (b.sinceMs ?? 0) ||
        a.nodeId.localeCompare(b.nodeId),
    )
    .filter((i) => (seen.has(i.nodeId) ? false : (seen.add(i.nodeId), true)));
}

export function splitVisible(items: InboxItem[], max = MAX_ROWS): { shown: InboxItem[]; more: number } {
  return { shown: items.slice(0, max), more: Math.max(0, items.length - max) };
}
