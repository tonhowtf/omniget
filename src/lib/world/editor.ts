// The home editor's state: a local draft of placements over what the server
// has published, with undo/redo and a saved copy per home. Pure functions
// and one class; nothing here touches Tauri or the canvas.

export type Space = 'outside' | 'inside';

export interface Placement {
  id: string;
  asset: string;
  xy: [number, number];
  space: Space;
  dir?: number;
  /** The world object id, for published placements only. */
  object_id?: number;
  /** A `picture/frame`'s image (SHA-256 of an uploaded asset). */
  picture?: string | null;
}

/** A house finish (`house_variant`): the workshop palette over the house sprite. */
export interface HouseFinish {
  id: number;
  key: string;
  tint: string;
}

/**
 * The enumeration the server publishes with `GET /homes/{id}/objects`
 * (`house_variants`); kept here only for a server that predates it. Same
 * order and colours as the workshop's `PALETTES`.
 */
export const HOUSE_FINISHES: HouseFinish[] = [
  { id: 0, key: 'terracotta', tint: '#ffffff' },
  { id: 1, key: 'breeze', tint: '#cddfee' },
  { id: 2, key: 'sand', tint: '#efd3b8' },
];

export function finishTint(finishes: HouseFinish[], variant: number | null | undefined): number {
  const f = finishes.find((x) => x.id === variant);
  const n = f ? parseInt(f.tint.replace('#', ''), 16) : NaN;
  return Number.isFinite(n) ? n : 0xffffff;
}

/** What an asset does in the world today (server `uses`): decoration unless listed. */
export type AssetUse = 'sit' | 'work' | 'picture';

/** What the server told us is published (from `GET /api/world/homes/{id}/objects`). */
export interface Published {
  revision: number;
  packageHash: string;
  houseVariant: number;
  objects: Placement[];
  catalogue: Record<Space, string[]>;
  finishes: HouseFinish[];
  uses: Record<string, AssetUse>;
}

export interface Draft {
  /** Objects added or moved since the published revision, by id. */
  placed: Record<string, Placement>;
  /** Published ids the draft removes. */
  removed: string[];
  houseVariant: number | null;
}

export type EditorStatus = 'clean' | 'draft' | 'publishing' | 'published' | 'conflict' | 'error';

export const FOOTPRINT: Record<string, [number, number]> = {
  'furniture/bed': [2, 2],
  'furniture/rug': [2, 2],
};

export function footprintOf(asset: string): [number, number] {
  return FOOTPRINT[asset] ?? [1, 1];
}

/** What the renderer draws for a catalogue asset in a space. */
export function runtimeKind(space: Space, asset: string): string {
  if (space === 'outside' || asset.startsWith('picture/')) return asset;
  const leaf = asset.split('/').pop() ?? '';
  return leaf === 'shelf' ? 'object/bookshelf' : `object/${leaf}`;
}

export function emptyDraft(): Draft {
  return { placed: {}, removed: [], houseVariant: null };
}

export function isEmpty(d: Draft): boolean {
  return Object.keys(d.placed).length === 0 && d.removed.length === 0 && d.houseVariant === null;
}

export function newId(): string {
  const n = Math.floor(Math.random() * 0xffffffff).toString(16).padStart(8, '0');
  return `draft:${Date.now().toString(36)}-${n}`;
}

/** The objects the screen should show: published minus removed plus draft. */
export function effective(published: Placement[], d: Draft): { placement: Placement; pending: boolean }[] {
  const out: { placement: Placement; pending: boolean }[] = [];
  for (const p of published) {
    if (d.removed.includes(p.id) || d.placed[p.id]) continue;
    out.push({ placement: p, pending: false });
  }
  for (const p of Object.values(d.placed)) out.push({ placement: p, pending: true });
  return out;
}

/** The body of `POST /api/world/homes/{id}/publish`. */
export function publishBody(pub: Published, d: Draft, key: string) {
  return {
    base_revision: pub.revision,
    package_hash: pub.packageHash,
    idempotency_key: key,
    house_variant: d.houseVariant ?? undefined,
    placed: Object.values(d.placed).map((p) => ({ id: p.id, asset: p.asset, xy: p.xy, space: p.space, dir: p.dir ?? 0, ...(p.picture ? { picture: p.picture } : {}) })),
    removed: d.removed,
  };
}

export class Editor {
  draft: Draft = emptyDraft();
  private past: Draft[] = [];
  private future: Draft[] = [];
  /** The idempotency key of the publish in flight, kept across a retry. */
  pendingKey: string | null = null;

  constructor(public onchange: () => void = () => {}) {}

  private edit(f: (d: Draft) => void): void {
    const next: Draft = structuredClone(this.draft);
    f(next);
    this.past.push(this.draft);
    if (this.past.length > 80) this.past.shift();
    this.future = [];
    this.draft = next;
    this.onchange();
  }

  place(asset: string, xy: [number, number], space: Space, picture?: string | null): string {
    const id = newId();
    this.edit((d) => {
      d.placed[id] = { id, asset, xy, space, dir: 0, ...(picture ? { picture } : {}) };
    });
    return id;
  }

  /** Change the image of a draft frame. */
  setPicture(id: string, picture: string): void {
    this.edit((d) => {
      if (d.placed[id]) d.placed[id] = { ...d.placed[id], picture };
    });
  }

  move(id: string, xy: [number, number]): void {
    this.edit((d) => {
      if (d.placed[id]) d.placed[id] = { ...d.placed[id], xy };
    });
  }

  /** Remove a draft object (forgotten) or a published one (marked removed). */
  remove(id: string, published: boolean): void {
    this.edit((d) => {
      if (d.placed[id]) delete d.placed[id];
      if (published && !d.removed.includes(id)) d.removed.push(id);
    });
  }

  /** Turn a draft object a quarter; published objects are re-placed by the canvas. */
  rotate(id: string): void {
    this.edit((d) => {
      if (d.placed[id]) d.placed[id] = { ...d.placed[id], dir: ((d.placed[id].dir ?? 0) + 1) % 4 };
    });
  }

  setHouseVariant(v: number | null): void {
    this.edit((d) => {
      d.houseVariant = v;
    });
  }

  undo(): void {
    const prev = this.past.pop();
    if (!prev) return;
    this.future.push(this.draft);
    this.draft = prev;
    this.onchange();
  }

  redo(): void {
    const next = this.future.pop();
    if (!next) return;
    this.past.push(this.draft);
    this.draft = next;
    this.onchange();
  }

  get canUndo(): boolean {
    return this.past.length > 0;
  }

  get canRedo(): boolean {
    return this.future.length > 0;
  }

  discard(): void {
    this.edit((d) => {
      d.placed = {};
      d.removed = [];
      d.houseVariant = null;
    });
    this.pendingKey = null;
  }

  /** After a successful publish the draft is folded into the published set. */
  published(): void {
    this.draft = emptyDraft();
    this.past = [];
    this.future = [];
    this.pendingKey = null;
    this.onchange();
  }

  /** Persist per home in the browser; a failure to store is not an error. */
  save(homeId: string): void {
    try {
      const key = `omniget.world.draft.${homeId}`;
      if (isEmpty(this.draft)) localStorage.removeItem(key);
      else localStorage.setItem(key, JSON.stringify({ draft: this.draft, pendingKey: this.pendingKey }));
    } catch {
      // storage unavailable
    }
  }

  restore(homeId: string): boolean {
    try {
      const raw = localStorage.getItem(`omniget.world.draft.${homeId}`);
      if (!raw) return false;
      const parsed = JSON.parse(raw) as { draft: Draft; pendingKey: string | null };
      if (!parsed?.draft || typeof parsed.draft !== 'object') return false;
      this.draft = { placed: parsed.draft.placed ?? {}, removed: parsed.draft.removed ?? [], houseVariant: parsed.draft.houseVariant ?? null };
      this.pendingKey = parsed.pendingKey ?? null;
      this.onchange();
      return true;
    } catch {
      return false;
    }
  }
}
