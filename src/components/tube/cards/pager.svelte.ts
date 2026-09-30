// A `Page` plus its continuation, for every infinite list of the video face.
import * as api from "$lib/tube/api";
import type { ContinuationKind } from "$lib/tube/api";
import type { Page, Shelf } from "$lib/tube/types";
import { errText } from "./util";

/** Append a continuation page: untitled shelves of the same kind extend the last one. */
export function mergePages(base: Page, more: Page): Page {
  const shelves = base.shelves.slice();
  for (const s of more.shelves) {
    const last = shelves[shelves.length - 1];
    if (last && !s.title && last.kind === s.kind) {
      shelves[shelves.length - 1] = { ...last, items: [...last.items, ...s.items] };
    } else {
      shelves.push({ ...s, id: `${s.id}~${shelves.length}` });
    }
  }
  return { ...base, shelves, continuation: more.continuation ?? null };
}

export class Pager {
  page = $state<Page | null>(null);
  error = $state<string | null>(null);
  loading = $state(false);
  loadingMore = $state(false);
  moreError = $state<string | null>(null);
  #gen = 0;
  kind: ContinuationKind;
  first: () => Promise<Page>;

  constructor(kind: ContinuationKind, first: () => Promise<Page>) {
    this.kind = kind;
    this.first = first;
  }

  get hasMore(): boolean {
    return !!this.page?.continuation;
  }

  get shelves(): Shelf[] {
    return this.page?.shelves ?? [];
  }

  /** (Re)load from the first page. `keep` leaves the old page on screen meanwhile. */
  async load(first?: () => Promise<Page>, keep = false) {
    if (first) this.first = first;
    const gen = ++this.#gen;
    this.loading = true;
    this.error = null;
    this.moreError = null;
    if (!keep) this.page = null;
    try {
      const p = await this.first();
      if (gen !== this.#gen) return;
      this.page = p;
    } catch (e) {
      if (gen !== this.#gen) return;
      this.error = errText(e);
    } finally {
      if (gen === this.#gen) this.loading = false;
    }
  }

  async more() {
    const token = this.page?.continuation;
    if (!token || this.loadingMore || this.loading) return;
    const gen = this.#gen;
    this.loadingMore = true;
    this.moreError = null;
    try {
      const next = await api.continuation(this.kind, token);
      if (gen !== this.#gen || !this.page) return;
      this.page = mergePages(this.page, next);
    } catch (e) {
      if (gen === this.#gen) this.moreError = errText(e);
    } finally {
      if (gen === this.#gen) this.loadingMore = false;
    }
  }
}
