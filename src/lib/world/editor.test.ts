import { describe, expect, it } from 'vitest';
import { Editor, HOUSE_FINISHES, finishTint, publishBody, runtimeKind, type Published } from './editor';

const pub: Published = {
  revision: 4,
  packageHash: 'pkg',
  houseVariant: 0,
  objects: [{ id: 'draft:old', asset: 'picture/frame', xy: [1, 1], space: 'outside', picture: 'c'.repeat(64), object_id: 9 }],
  catalogue: { outside: ['picture/frame'], inside: ['picture/frame'] },
  finishes: HOUSE_FINISHES,
  uses: {},
};

describe('house finishes', () => {
  it('match the workshop palette and fall back to white', () => {
    expect(HOUSE_FINISHES.map((f) => f.tint)).toEqual(['#ffffff', '#cddfee', '#efd3b8']);
    expect(finishTint(HOUSE_FINISHES, 1)).toBe(0xcddfee);
    expect(finishTint(HOUSE_FINISHES, 7)).toBe(0xffffff);
    expect(finishTint([], null)).toBe(0xffffff);
  });
});

describe('picture frames in the draft', () => {
  it('a frame carries its picture to the publish body; other objects do not', () => {
    const e = new Editor();
    const a = e.place('picture/frame', [2, 3], 'inside', 'a'.repeat(64));
    e.place('prop/bench', [4, 4], 'outside');
    e.setHouseVariant(2);
    const body = publishBody(pub, e.draft, 'k');
    expect(body.house_variant).toBe(2);
    const frame = body.placed.find((p) => p.id === a)!;
    expect(frame.picture).toBe('a'.repeat(64));
    expect(body.placed.find((p) => p.asset === 'prop/bench')).not.toHaveProperty('picture');
    e.setPicture(a, 'b'.repeat(64));
    expect(publishBody(pub, e.draft, 'k').placed.find((p) => p.id === a)!.picture).toBe('b'.repeat(64));
    e.undo();
    expect(e.draft.placed[a].picture).toBe('a'.repeat(64));
  });

  it('a frame is the same kind inside and outside', () => {
    expect(runtimeKind('inside', 'picture/frame')).toBe('picture/frame');
    expect(runtimeKind('inside', 'furniture/shelf')).toBe('object/bookshelf');
  });

  it('the draft (finish included) survives a reload, the conflict path keeps it', () => {
    const store = new Map<string, string>();
    (globalThis as unknown as { localStorage: Storage }).localStorage = {
      getItem: (k: string) => store.get(k) ?? null,
      setItem: (k: string, v: string) => void store.set(k, v),
      removeItem: (k: string) => void store.delete(k),
    } as Storage;
    const e = new Editor();
    e.setHouseVariant(1);
    e.place('picture/frame', [1, 2], 'outside', 'd'.repeat(64));
    e.pendingKey = 'retry-key';
    e.save('h1');
    const back = new Editor();
    expect(back.restore('h1')).toBe(true);
    expect(back.draft.houseVariant).toBe(1);
    expect(Object.values(back.draft.placed)[0].picture).toBe('d'.repeat(64));
    expect(back.pendingKey).toBe('retry-key');
  });
});
