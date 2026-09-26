import { describe, expect, it } from 'vitest';
import {
  FRAME_LOADING,
  FRAME_PLACEHOLDER,
  PAGE_SIZE,
  PAGE_SLOTS,
  PICTURE_BOX,
  PictureStore,
  SLOT_H,
  SLOT_W,
  fitPicture,
  frameForKind,
  frameGeometry,
  pictureFrameId,
  pictureHash,
  slotOrigin,
  type PictureFetch,
} from './pictures';

const H1 = 'a'.repeat(64);
const H2 = 'b'.repeat(64);

function fakeBitmap(w = 10, h = 10) {
  let closed = false;
  return {
    width: w,
    height: h,
    close() {
      closed = true;
    },
    get closed() {
      return closed;
    },
  } as unknown as ImageBitmap & { closed: boolean };
}

function flush() {
  return new Promise((r) => setTimeout(r, 0));
}

function store(answers: Record<string, PictureFetch | (() => PictureFetch)>) {
  const calls: string[] = [];
  const s = new PictureStore({
    fetch: async (h) => {
      calls.push(h);
      const a = answers[h];
      return typeof a === 'function' ? a() : (a ?? { ok: false, code: 'ERR_CITY_UNREACHABLE' });
    },
    decode: async () => fakeBitmap(),
  });
  return { s, calls };
}

describe('picture kinds', () => {
  it('reads the hash from the kind and nothing else', () => {
    expect(pictureHash(`picture/${H1}`)).toBe(H1);
    expect(pictureHash('picture/removed')).toBeNull();
    expect(pictureHash('picture/ABC')).toBeNull();
    expect(pictureHash('prop/flowers')).toBeNull();
  });

  it('draws loading, the picture or the placeholder by state', () => {
    const { s } = store({});
    expect(frameForKind('prop/bench', s, new Set())).toBeNull();
    expect(frameForKind('picture/removed', s, new Set())).toBe(FRAME_PLACEHOLDER);
    expect(frameForKind(`picture/${H1}`, s, new Set())).toBe(FRAME_LOADING);
    s.entries.set(H1, { hash: H1, state: 'ready', image: fakeBitmap(), lastWanted: 0 });
    expect(frameForKind(`picture/${H1}`, s, new Set([H1]))).toBe(pictureFrameId(H1));
    // Ready but not on the page yet (the page is recomposed next frame).
    expect(frameForKind(`picture/${H1}`, s, new Set())).toBe(FRAME_LOADING);
    s.drop(H1, 'denied');
    expect(frameForKind(`picture/${H1}`, s, new Set([H1]))).toBe(FRAME_PLACEHOLDER);
  });
});

describe('frame geometry', () => {
  it('fits in the box, keeps the aspect and does not upscale past it', () => {
    expect(fitPicture(512, 256)).toEqual({ w: PICTURE_BOX, h: PICTURE_BOX / 2 });
    expect(fitPicture(100, 400)).toEqual({ w: 10, h: PICTURE_BOX });
    expect(fitPicture(8, 8)).toEqual({ w: PICTURE_BOX, h: PICTURE_BOX });
  });

  it('anchors on the easel feet, centred', () => {
    const g = frameGeometry({ w: 40, h: 20 });
    expect(g.w).toBe(46);
    expect(g.pivotX).toBe(23);
    expect(g.pivotY).toBe(g.h - 1);
    expect(g.inner).toEqual({ x: 3, y: 3, w: 40, h: 20 });
  });

  it('every slot stays on the page', () => {
    expect(PAGE_SLOTS).toBeGreaterThan(40);
    const last = slotOrigin(PAGE_SLOTS - 1);
    expect(last.x + SLOT_W).toBeLessThanOrEqual(PAGE_SIZE);
    expect(last.y + SLOT_H).toBeLessThanOrEqual(PAGE_SIZE);
  });
});

describe('PictureStore', () => {
  it('loads what is wanted once, and moves the version when ready', async () => {
    const { s, calls } = store({ [H1]: { ok: true, bytes: new Uint8Array([1]) } });
    const v0 = s.version;
    s.want([H1]);
    s.want([H1]);
    await flush();
    expect(calls).toEqual([H1]);
    expect(s.stateOf(H1)).toBe('ready');
    expect(s.version).toBeGreaterThan(v0);
  });

  it('a removal (410) or a refusal (403) forgets the decoded image', async () => {
    let removed = false;
    const { s } = store({
      [H1]: () => (removed ? { ok: false, code: 'ERR_WORLD_ASSET_REMOVED' } : { ok: true, bytes: new Uint8Array([1]) }),
      [H2]: { ok: false, code: 'ERR_WORLD_ASSET_PRIVATE' },
    });
    s.want([H1, H2]);
    await flush();
    const img = s.entries.get(H1)!.image as unknown as { closed: boolean };
    expect(s.stateOf(H1)).toBe('ready');
    expect(s.stateOf(H2)).toBe('denied');
    expect(s.entries.get(H2)!.image).toBeNull();
    // Reconnected: ask again; the server now says it is gone.
    removed = true;
    s.revalidate();
    await flush();
    expect(s.stateOf(H1)).toBe('gone');
    expect(s.entries.get(H1)!.image).toBeNull();
    expect(img.closed).toBe(true);
  });

  it('a network failure during revalidation keeps the picture shown', async () => {
    let down = false;
    const { s } = store({ [H1]: () => (down ? { ok: false, code: 'ERR_CITY_UNREACHABLE' } : { ok: true, bytes: new Uint8Array([1]) }) });
    s.want([H1]);
    await flush();
    down = true;
    s.revalidate();
    await flush();
    expect(s.stateOf(H1)).toBe('ready');
  });

  it('drop from the world (placeholder kind) wins over an image in cache', async () => {
    const { s } = store({ [H1]: { ok: true, bytes: new Uint8Array([1]) } });
    s.want([H1]);
    await flush();
    s.drop(H1, 'gone');
    s.want([H1]);
    await flush();
    expect(s.stateOf(H1)).toBe('gone');
  });

  it('trim releases the least recently wanted', async () => {
    let t = 0;
    const s = new PictureStore({ fetch: async () => ({ ok: true, bytes: new Uint8Array([1]) }), decode: async () => fakeBitmap(), now: () => t });
    t = 1;
    s.want([H1]);
    t = 2;
    s.want([H2]);
    await flush();
    s.trim(1);
    expect(s.stateOf(H1)).toBeNull();
    expect(s.stateOf(H2)).toBe('ready');
  });
});
