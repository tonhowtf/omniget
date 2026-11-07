import type { BoardEntry, SortMode } from "./types";

function fold(text: string): string {
  return text.normalize("NFD").replace(/\p{Diacritic}/gu, "").toLowerCase();
}

export function filterBoards(boards: readonly BoardEntry[], query: string): BoardEntry[] {
  const words = fold(query).split(/\s+/).filter(Boolean);
  if (words.length === 0) return [...boards];
  return boards.filter((b) => {
    const name = fold(b.name);
    return words.every((w) => name.includes(w));
  });
}

const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });

export function sortBoards(boards: readonly BoardEntry[], mode: SortMode): BoardEntry[] {
  const copy = [...boards];
  if (mode === "name") return copy.sort((a, b) => collator.compare(a.name, b.name) || b.modified_ms - a.modified_ms);
  return copy.sort((a, b) => b.modified_ms - a.modified_ms || collator.compare(a.name, b.name));
}

export function visibleBoards(boards: readonly BoardEntry[], query: string, mode: SortMode): BoardEntry[] {
  return sortBoards(filterBoards(boards, query), mode);
}

export function upsert(boards: readonly BoardEntry[], entry: BoardEntry, replacing?: string): BoardEntry[] {
  const without = boards.filter((b) => b.path !== entry.path && b.path !== replacing);
  return [entry, ...without];
}

export function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value < 10 ? value.toFixed(1) : Math.round(value)} ${units[unit]}`;
}

export type Ago = { unit: "now" | "minute" | "hour" | "day" | "date"; value: number };

export function ago(then: number, now: number): Ago {
  const diff = Math.max(0, now - then);
  const minutes = Math.floor(diff / 60_000);
  if (minutes < 1) return { unit: "now", value: 0 };
  if (minutes < 60) return { unit: "minute", value: minutes };
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return { unit: "hour", value: hours };
  const days = Math.floor(hours / 24);
  if (days < 7) return { unit: "day", value: days };
  return { unit: "date", value: then };
}
