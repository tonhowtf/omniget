import * as api from "$lib/tube/api";
import type { DeArrow } from "$lib/tube/types";

export const dearrow = $state({
  map: {} as Record<string, DeArrow | null>,
  original: {} as Record<string, boolean>,
});

const pending = new Set<string>();
const asked = new Set<string>();
let timer: ReturnType<typeof setTimeout> | null = null;

async function flush() {
  timer = null;
  const ids = [...pending];
  pending.clear();
  for (let i = 0; i < ids.length; i += 50) {
    const chunk = ids.slice(i, i + 50);
    try {
      const res = await api.dearrow(chunk);
      for (const id of chunk) dearrow.map[id] = res[id] ?? null;
    } catch {
      for (const id of chunk) asked.delete(id);
    }
  }
}

export function wantDearrow(id: string) {
  if (asked.has(id)) return;
  asked.add(id);
  pending.add(id);
  timer ??= setTimeout(flush, 120);
}

export function toggleOriginal(id: string) {
  dearrow.original[id] = !dearrow.original[id];
}
