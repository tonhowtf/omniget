// Pictures users publish in the city: the store that fetches and forgets
// them, and the runtime atlas page they are drawn on.
//
// A published frame arrives as a world object whose kind is
// `picture/<sha256>`; `picture/removed` is a frame whose image was taken
// down. The canvas asks the store for the hashes it sees, the store fetches
// the bytes through the server (which decides who may read them), decodes
// them small, and the page composer draws every ready picture into one
// canvas that the renderer takes as its dynamic page. Nothing here keeps a
// picture the server stopped serving: a 410 or 403 drops the decoded image
// and the frame shows the placeholder.

/** The catalogue asset of a standing picture frame. */
export const PICTURE_ASSET = 'picture/frame';
export const PICTURE_REMOVED = 'picture/removed';
/** Frame ids on the dynamic page. */
export const FRAME_PLACEHOLDER = 'pic/placeholder';
export const FRAME_LOADING = 'pic/loading';

/** The picture area inside the frame, in world pixels at zoom 1 (a tile is 64 wide). */
export const PICTURE_BOX = 40;
/** Moulding around the picture. */
export const FRAME_BORDER = 3;
/** The easel under the frame. */
export const EASEL_H = 14;
/** One slot of the page: the biggest frame fits with a margin for filtering. */
export const SLOT_W = PICTURE_BOX + FRAME_BORDER * 2 + 4;
export const SLOT_H = PICTURE_BOX + FRAME_BORDER * 2 + EASEL_H + 4;
export const PAGE_SIZE = 512;
/** Slots on a page, two of them reserved for the placeholder and the loading frame. */
const PAGE_MARGIN = 2;
const PAGE_COLS = Math.floor((PAGE_SIZE - PAGE_MARGIN) / SLOT_W);
export const PAGE_SLOTS = PAGE_COLS * Math.floor((PAGE_SIZE - PAGE_MARGIN) / SLOT_H);
/** The backing a picture sits on; a transparent image shows it through. */
export const BACKING = '#f4efe4';
const MOULDING = '#6b4a2b';
const MOULDING_LIGHT = '#8a6440';

/** The hash a world object kind shows, or null for anything else (including the placeholder). */
export function pictureHash(kind: string): string | null {
  if (!kind.startsWith('picture/')) return null;
  const h = kind.slice(8);
  return /^[0-9a-f]{64}$/.test(h) ? h : null;
}

export function isPictureKind(kind: string): boolean {
  return kind.startsWith('picture/');
}

/** Frame id of a ready picture. */
export function pictureFrameId(hash: string): string {
  return `pic/${hash}`;
}

/** Picture size inside the box, keeping the aspect; never upscales past the box. */
export function fitPicture(w: number, h: number): { w: number; h: number } {
  if (w <= 0 || h <= 0) return { w: PICTURE_BOX, h: PICTURE_BOX };
  const s = Math.min(PICTURE_BOX / w, PICTURE_BOX / h);
  return { w: Math.max(1, Math.round(w * s)), h: Math.max(1, Math.round(h * s)) };
}

/**
 * Geometry of one framed picture: the sprite is the moulding plus the easel,
 * anchored at the middle of the easel's feet, which the scene puts on the
 * tile centre. `inner` is where the image goes, relative to the sprite.
 */
export function frameGeometry(pic: { w: number; h: number }): {
  w: number;
  h: number;
  pivotX: number;
  pivotY: number;
  inner: { x: number; y: number; w: number; h: number };
} {
  const w = pic.w + FRAME_BORDER * 2;
  const h = pic.h + FRAME_BORDER * 2 + EASEL_H;
  return {
    w,
    h,
    pivotX: Math.floor(w / 2),
    pivotY: h - 1,
    inner: { x: FRAME_BORDER, y: FRAME_BORDER, w: pic.w, h: pic.h },
  };
}

/** Top-left of slot `i` on the page. */
export function slotOrigin(i: number): { x: number; y: number } {
  return { x: (i % PAGE_COLS) * SLOT_W + PAGE_MARGIN, y: Math.floor(i / PAGE_COLS) * SLOT_H + PAGE_MARGIN };
}

export type PictureState = 'loading' | 'ready' | 'gone' | 'denied' | 'error';

export interface PictureEntry {
  hash: string;
  state: PictureState;
  /** Decoded and already reduced to the box. Only while `ready`. */
  image: ImageBitmap | null;
  /** When it was last asked for (ms), for the least-recently-used drop. */
  lastWanted: number;
}

/** What loading a hash can end in; `bytes` are the file as served. */
export type PictureFetch =
  | { ok: true; bytes: Uint8Array | ArrayBuffer }
  | { ok: false; code: 'ERR_WORLD_ASSET_REMOVED' | 'ERR_WORLD_ASSET_PRIVATE' | string };

export interface PictureStoreOpts {
  fetch: (hash: string) => Promise<PictureFetch>;
  /** Bytes → a bitmap no bigger than the box. */
  decode: (bytes: Uint8Array | ArrayBuffer) => Promise<ImageBitmap>;
  now?: () => number;
  /** Retry an `error` (network) after this long. */
  retryMs?: number;
  maxConcurrent?: number;
}

/**
 * The pictures this client knows. `version` moves whenever something that
 * changes the page happens (a picture ready, dropped or forgotten), so the
 * canvas recomposes only then.
 */
export class PictureStore {
  readonly entries = new Map<string, PictureEntry>();
  version = 0;
  private inFlight = 0;
  private queue: string[] = [];
  private errorAt = new Map<string, number>();
  private disposed = false;

  constructor(private opts: PictureStoreOpts) {}

  private now(): number {
    return this.opts.now ? this.opts.now() : Date.now();
  }

  /** The hashes on screen now: unknown ones start loading. */
  want(hashes: Iterable<string>): void {
    const t = this.now();
    for (const h of hashes) {
      const e = this.entries.get(h);
      if (e) {
        e.lastWanted = t;
        if (e.state === 'error' && t - (this.errorAt.get(h) ?? 0) > (this.opts.retryMs ?? 15_000)) {
          e.state = 'loading';
          this.enqueue(h);
        }
        continue;
      }
      this.entries.set(h, { hash: h, state: 'loading', image: null, lastWanted: t });
      this.enqueue(h);
    }
  }

  private enqueue(h: string): void {
    if (!this.queue.includes(h)) this.queue.push(h);
    this.pump();
  }

  private pump(): void {
    const max = this.opts.maxConcurrent ?? 3;
    while (!this.disposed && this.inFlight < max && this.queue.length > 0) {
      const h = this.queue.shift()!;
      this.inFlight++;
      void this.load(h).finally(() => {
        this.inFlight--;
        this.pump();
      });
    }
  }

  private async load(h: string): Promise<void> {
    let res: PictureFetch;
    try {
      res = await this.opts.fetch(h);
    } catch (e) {
      res = { ok: false, code: e instanceof Error ? e.message : String(e) };
    }
    const e = this.entries.get(h);
    if (!e || this.disposed) return;
    if (!res.ok) {
      if (res.code.includes('ERR_WORLD_ASSET_REMOVED')) this.drop(h, 'gone');
      else if (res.code.includes('ERR_WORLD_ASSET_PRIVATE') || res.code.includes('FORBIDDEN')) this.drop(h, 'denied');
      else {
        this.errorAt.set(h, this.now());
        // A failed revalidation keeps what was already shown.
        if (e.state !== 'ready') {
          e.state = 'error';
          this.version++;
        }
      }
      return;
    }
    try {
      const image = await this.opts.decode(res.bytes);
      if (this.disposed || this.entries.get(h) !== e) {
        image.close?.();
        return;
      }
      e.image?.close?.();
      e.image = image;
      e.state = 'ready';
      this.version++;
    } catch {
      e.state = 'error';
      this.errorAt.set(h, this.now());
      this.version++;
    }
  }

  /** The server said no (removed, not allowed): forget the image, keep the verdict. */
  drop(h: string, state: 'gone' | 'denied'): void {
    const e = this.entries.get(h) ?? { hash: h, state, image: null, lastWanted: this.now() };
    e.image?.close?.();
    e.image = null;
    e.state = state;
    this.entries.set(h, e);
    this.queue = this.queue.filter((q) => q !== h);
    this.version++;
  }

  /**
   * Ask the server again about everything decoded (after a reconnection:
   * something may have been removed while we were away). A picture keeps
   * showing until the answer says otherwise.
   */
  revalidate(): void {
    for (const e of this.entries.values()) {
      if (e.state === 'ready' || e.state === 'error' || e.state === 'denied') this.enqueue(e.hash);
    }
  }

  /** Least-recently-wanted ready pictures beyond `keep` are released. */
  trim(keep: number): void {
    const ready = [...this.entries.values()].filter((e) => e.state === 'ready').sort((a, b) => b.lastWanted - a.lastWanted);
    for (const e of ready.slice(keep)) {
      e.image?.close?.();
      this.entries.delete(e.hash);
      this.version++;
    }
  }

  stateOf(h: string): PictureState | null {
    return this.entries.get(h)?.state ?? null;
  }

  dispose(): void {
    this.disposed = true;
    for (const e of this.entries.values()) e.image?.close?.();
    this.entries.clear();
    this.queue = [];
  }
}

/** The frame id a picture object draws with, whatever its state. */
export function frameForKind(kind: string, store: PictureStore | null, onPage: ReadonlySet<string>): string | null {
  if (!isPictureKind(kind)) return null;
  const h = pictureHash(kind);
  if (!h) return FRAME_PLACEHOLDER;
  const state = store?.stateOf(h) ?? 'loading';
  if (state === 'ready' && onPage.has(h)) return pictureFrameId(h);
  if (state === 'loading' || state === 'ready') return FRAME_LOADING;
  return FRAME_PLACEHOLDER;
}

type Ctx2D = CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D;

function drawMoulding(c: Ctx2D, ox: number, oy: number, g: ReturnType<typeof frameGeometry>): void {
  const fw = g.w;
  const fh = g.h - EASEL_H;
  // Easel: two legs and a crossbar, behind the frame.
  c.fillStyle = MOULDING;
  const legY = oy + fh - 4;
  c.fillRect(ox + Math.floor(fw * 0.25) - 1, legY, 2, EASEL_H + 3);
  c.fillRect(ox + Math.ceil(fw * 0.75) - 1, legY, 2, EASEL_H + 3);
  c.fillRect(ox + Math.floor(fw * 0.25), oy + g.h - 5, Math.ceil(fw * 0.5), 2);
  // The moulding: dark outside, a lighter inner bevel.
  c.fillRect(ox, oy, fw, fh);
  c.fillStyle = MOULDING_LIGHT;
  c.fillRect(ox + 1, oy + 1, fw - 2, fh - 2);
  c.fillStyle = BACKING;
  c.fillRect(ox + g.inner.x, oy + g.inner.y, g.inner.w, g.inner.h);
}

/**
 * Draws the page: placeholder, loading frame, then every ready picture the
 * page has room for (most recently wanted first). Returns the frames, and
 * which hashes made it onto the page.
 */
export function composePicturePage(
  c: Ctx2D,
  store: PictureStore,
): { frames: Record<string, { x: number; y: number; w: number; h: number; pivotX: number; pivotY: number }>; onPage: Set<string> } {
  c.clearRect(0, 0, PAGE_SIZE, PAGE_SIZE);
  c.imageSmoothingEnabled = true;
  const frames: Record<string, { x: number; y: number; w: number; h: number; pivotX: number; pivotY: number }> = {};
  const onPage = new Set<string>();
  const put = (id: string, slot: number, g: ReturnType<typeof frameGeometry>) => {
    const o = slotOrigin(slot);
    frames[id] = { x: o.x, y: o.y, w: g.w, h: g.h, pivotX: g.pivotX, pivotY: g.pivotY };
    return o;
  };
  // Placeholder: a frame with a dim cross, the same size as a square picture.
  const square = frameGeometry({ w: PICTURE_BOX, h: PICTURE_BOX });
  let o = put(FRAME_PLACEHOLDER, 0, square);
  drawMoulding(c, o.x, o.y, square);
  c.fillStyle = '#c9c2b4';
  c.fillRect(o.x + square.inner.x, o.y + square.inner.y, square.inner.w, square.inner.h);
  c.strokeStyle = '#8f8778';
  c.lineWidth = 2;
  c.beginPath();
  c.moveTo(o.x + square.inner.x + 8, o.y + square.inner.y + 8);
  c.lineTo(o.x + square.inner.x + square.inner.w - 8, o.y + square.inner.y + square.inner.h - 8);
  c.moveTo(o.x + square.inner.x + square.inner.w - 8, o.y + square.inner.y + 8);
  c.lineTo(o.x + square.inner.x + 8, o.y + square.inner.y + square.inner.h - 8);
  c.stroke();
  // Loading: the empty backing in its frame.
  o = put(FRAME_LOADING, 1, square);
  drawMoulding(c, o.x, o.y, square);

  const ready = [...store.entries.values()].filter((e) => e.state === 'ready' && e.image).sort((a, b) => b.lastWanted - a.lastWanted);
  let slot = 2;
  for (const e of ready) {
    if (slot >= PAGE_SLOTS) break;
    const img = e.image!;
    const g = frameGeometry(fitPicture(img.width, img.height));
    o = put(pictureFrameId(e.hash), slot++, g);
    drawMoulding(c, o.x, o.y, g);
    c.drawImage(img as CanvasImageSource, o.x + g.inner.x, o.y + g.inner.y, g.inner.w, g.inner.h);
    onPage.add(e.hash);
  }
  return { frames, onPage };
}

/** Decodes and reduces to the box in one step, where the webview can. */
export async function decodePicture(bytes: Uint8Array | ArrayBuffer): Promise<ImageBitmap> {
  const blob = new Blob([bytes as BlobPart]);
  const full = await createImageBitmap(blob);
  const fit = fitPicture(full.width, full.height);
  if (fit.w === full.width && fit.h === full.height) return full;
  try {
    const small = await createImageBitmap(full, { resizeWidth: fit.w, resizeHeight: fit.h, resizeQuality: 'high' });
    full.close();
    return small;
  } catch {
    return full;
  }
}
