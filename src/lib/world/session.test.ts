// The session contract, driven by the fake. The blobs here are built with the
// same test writer the codec test uses, because the point of these cases is
// the state machine around the channel, not the bytes.

import { describe, expect, it } from 'vitest';
import { KIND_DIFF, KIND_SNAPSHOT } from './codec';
import { createFakeSession, toBytes, type WorldInput } from './session';
import { WorldState } from './state';

/** Minimal encoder, enough for a snapshot with one agent and a diff moving it. */
function bytes(parts: number[]): Uint8Array {
  return new Uint8Array(parts);
}

function varint(v: number): number[] {
  const out: number[] = [];
  let n = v;
  for (;;) {
    const b = n % 128;
    n = Math.floor(n / 128);
    if (n === 0) {
      out.push(b);
      return out;
    }
    out.push(b | 0x80);
  }
}

const zigzag = (v: number) => varint(((v << 1) ^ (v >> 31)) >>> 0);
const header = (kind: number) => [0x4f, 0x47, 0x57, 0x31, kind, 1];
const table = (items: string[]) => {
  const out = varint(items.length);
  for (const s of items) {
    const raw = [...Buffer.from(s, 'utf-8')];
    out.push(...varint(raw.length), ...raw);
  }
  return out;
};

function snapshotBlob(tick: number, mapHash: number): Uint8Array {
  return bytes([
    ...header(KIND_SNAPSHOT),
    ...varint(tick),
    ...varint(1), // seed
    ...varint(0), // rng
    ...varint(mapHash),
    0, // active
    ...table(['Omni']),
    ...varint(1), // one agent
    ...varint(1), // id
    ...varint(0), // name index
    ...zigzag(256),
    ...zigzag(512),
    ...zigzag(0),
    0, // dir
    0, // anim
    255, // energy
    0, // activity idle
    ...varint(0),
    ...varint(0), // no objects
  ]);
}

function moveBlob(from: number, to: number, mapHash: number, x: number): Uint8Array {
  return bytes([
    ...header(KIND_DIFF),
    ...varint(from),
    ...varint(to),
    ...varint(mapHash),
    ...varint(0),
    0,
    ...table([]),
    ...varint(1), // one delta
    ...varint(1), // id
    0x01, // POS
    ...zigzag(x),
    ...zigzag(512),
    ...zigzag(0),
    ...varint(0), // no objects
    ...varint(0), // no events
  ]);
}

describe('session: bytes off the channel', () => {
  it('accepts every shape Tauri may hand a Vec<u8> over as', () => {
    const raw = new Uint8Array([1, 2, 3]);
    expect(toBytes(raw)).toEqual(raw);
    expect(toBytes(raw.buffer)).toEqual(raw);
    expect(toBytes([1, 2, 3])).toEqual(raw);
    expect(toBytes({ data: [1, 2, 3] })).toEqual(raw);
    expect(() => toBytes(null)).toThrow();
  });
});

describe('session: the fake drives the whole route state machine', () => {
  const HASH = 77;

  it('opens with a snapshot and then applies diffs', async () => {
    const blobs = [
      snapshotBlob(10, HASH),
      moveBlob(10, 11, HASH, 512),
      moveBlob(11, 12, HASH, 768),
    ];
    const session = createFakeSession(blobs);
    const state = new WorldState();
    expect(await session.exists()).toBe(true);
    const opened = await session.open(
      (blob) => {
        if (blob.kind === 'snapshot') state.applySnapshot(blob.snapshot, 0);
        else state.applyDiff(blob.diff, 100);
      },
      () => {},
    );
    expect(opened.blob.kind).toBe('snapshot');
    if (opened.blob.kind === 'snapshot') state.applySnapshot(opened.blob.snapshot, 0);
    expect(state.tick).toBe(10);
    expect(session.advance()).toBe(true);
    expect(state.tick).toBe(11);
    expect(session.advance()).toBe(true);
    expect(state.tick).toBe(12);
    expect(session.advance()).toBe(false);
    expect(state.agents.get(1)!.track.toX).toBe(3);
  });

  it('a diff from another house is refused and does not move the tick', async () => {
    const session = createFakeSession([snapshotBlob(10, HASH), moveBlob(10, 11, 999, 512)]);
    const state = new WorldState();
    const codes: string[] = [];
    const opened = await session.open(
      (blob) => {
        const r =
          blob.kind === 'snapshot'
            ? state.applySnapshot(blob.snapshot, 0)
            : state.applyDiff(blob.diff, 100);
        if (!r.ok) codes.push(r.code!);
      },
      (code) => codes.push(code),
    );
    if (opened.blob.kind === 'snapshot') state.applySnapshot(opened.blob.snapshot, 0);
    session.advance();
    expect(codes).toEqual(['ERR_WORLD_MAP_MISMATCH']);
    expect(state.tick).toBe(10);
    expect(state.needsResync).toBe(true);
  });

  it('a corrupt blob reaches the error callback, not the state', async () => {
    const session = createFakeSession([snapshotBlob(10, HASH)]);
    const state = new WorldState();
    const errors: string[] = [];
    const opened = await session.open(
      (blob) => {
        if (blob.kind === 'snapshot') state.applySnapshot(blob.snapshot, 0);
      },
      (code) => errors.push(code),
    );
    if (opened.blob.kind === 'snapshot') state.applySnapshot(opened.blob.snapshot, 0);
    session.push(new Uint8Array([1, 2, 3, 4, 5, 6]));
    expect(errors).toHaveLength(1);
    expect(errors[0]).toContain('ERR_WORLD_BAD_MAGIC');
    expect(state.tick).toBe(10);
  });

  it('a resync replays from the snapshot', async () => {
    const session = createFakeSession([snapshotBlob(10, HASH), moveBlob(10, 11, HASH, 512)]);
    const state = new WorldState();
    const opened = await session.open(
      (blob) => {
        if (blob.kind === 'snapshot') state.applySnapshot(blob.snapshot, 0);
        else state.applyDiff(blob.diff, 100);
      },
      () => {},
    );
    if (opened.blob.kind === 'snapshot') state.applySnapshot(opened.blob.snapshot, 0);
    session.advance();
    expect(state.tick).toBe(11);
    await session.resync();
    expect(state.tick).toBe(10);
    expect(state.needsResync).toBe(false);
  });

  it('records what the route sends and what it says about visibility', async () => {
    const session = createFakeSession([snapshotBlob(10, HASH)]);
    const sleeps: string[] = [];
    session.onSleepState((s) => sleeps.push(s));
    const input: WorldInput = { type: 'move', ent: 1, to: [4, 5] };
    await session.send(input);
    await session.send({ type: 'place_object', object: 2, kind: 'object/lamp', tile: [4, 2], dir: 0, slot: 'bedroom-lamp' });
    await session.setVisible(false);
    await session.setVisible(true);
    expect(session.sent).toHaveLength(2);
    expect(session.sent[0]).toEqual(input);
    expect(sleeps).toEqual(['dozing', 'active']);
    expect(session.visible).toBe(true);
  });

  it('a world that does not exist yet says so instead of opening', async () => {
    const session = createFakeSession([]);
    expect(await session.exists()).toBe(false);
    await session.create();
    expect(await session.exists()).toBe(true);
    await expect(session.open(() => {}, () => {})).rejects.toThrow();
  });

  it('deleting the world closes it', async () => {
    const session = createFakeSession([snapshotBlob(10, HASH)]);
    await session.open(() => {}, () => {});
    expect(session.opened).toBe(true);
    await session.delete();
    expect(session.opened).toBe(false);
    expect(await session.exists()).toBe(false);
  });
});
