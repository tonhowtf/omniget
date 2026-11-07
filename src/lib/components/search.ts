import { HIDDEN_TYPES } from "./registry";
import type { ItemSummary } from "./types";

export type SearchFilters = {
  query: string;
  registryId: string | null;
  type: string | null;
};

function norm(s: string): string {
  return s
    .toLowerCase()
    .normalize("NFKD")
    .replace(/[\u0300-\u036f]/g, "")
    .replace(/[_\-./]+/g, " ")
    .trim();
}

function tokenScore(item: ItemSummary, token: string): number {
  const name = norm(item.name.replace(/([a-z])([A-Z])/g, "$1 $2"));
  const title = norm(item.title);
  const compactName = name.replace(/\s+/g, "");
  if (compactName === token || name === token) return 100;
  if (title === token) return 90;
  if (compactName.startsWith(token) || name.startsWith(token)) return 80;
  if (title.startsWith(token)) return 70;
  if (name.split(" ").some((w) => w.startsWith(token))) return 65;
  if (compactName.includes(token)) return 60;
  if (title.includes(token)) return 50;
  if (item.categories.some((c) => norm(c).includes(token))) return 35;
  const desc = norm(item.description);
  if (desc.split(/\s+/).some((w) => w.startsWith(token))) return 30;
  if (desc.includes(token)) return 20;
  return 0;
}

export function scoreItem(item: ItemSummary, query: string): number {
  const tokens = norm(query).split(/\s+/).filter(Boolean);
  if (tokens.length === 0) return 1;
  let total = 0;
  for (const token of tokens) {
    const s = tokenScore(item, token);
    if (s === 0) return 0;
    total += s;
  }
  return total;
}

export function search(items: ItemSummary[], filters: SearchFilters, registryOrder: string[] = []): ItemSummary[] {
  const order = new Map(registryOrder.map((id, i) => [id, i]));
  const scored: { item: ItemSummary; score: number }[] = [];
  for (const item of items) {
    if (filters.registryId && item.registryId !== filters.registryId) continue;
    if (filters.type ? item.type !== filters.type : HIDDEN_TYPES.has(item.type)) continue;
    const score = scoreItem(item, filters.query);
    if (score > 0) scored.push({ item, score });
  }
  scored.sort(
    (a, b) =>
      b.score - a.score ||
      (order.get(a.item.registryId) ?? 99) - (order.get(b.item.registryId) ?? 99) ||
      a.item.name.localeCompare(b.item.name),
  );
  return scored.map((s) => s.item);
}

export function typeCounts(items: ItemSummary[], registryId: string | null): [string, number][] {
  const counts = new Map<string, number>();
  for (const item of items) {
    if (registryId && item.registryId !== registryId) continue;
    counts.set(item.type, (counts.get(item.type) ?? 0) + 1);
  }
  return [...counts.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]));
}
