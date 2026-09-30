// Residents from the server: the same city session the 2D canvas uses
// (`$lib/world/city`, `/world/v1`), decoded into a WorldState and sampled
// with the same interpolation, then handed to the 3D people as positions in
// server tile units. The 3D city and the server's region ids are different
// grids; positions map 1:1 onto region [0, 0] until the server speaks the
// 3D regions (M8).

import { createCitySession, type CityFrame } from '$lib/world/city';
import { sample } from '$lib/world/interp';
import { WorldState } from '$lib/world/state';
import type { ServerAgent } from './people';

export async function startServerResidents(
  opts: { city: string; server: string | null },
  onAgents: (agents: ServerAgent[]) => void,
): Promise<() => void> {
  const session = createCitySession({ city: opts.city, server: opts.server });
  let world = new WorldState();
  let region = '';
  let timer: ReturnType<typeof setInterval> | null = null;
  const prev = new Map<number, { x: number; y: number; yaw: number }>();

  const onFrame = (frame: CityFrame) => {
    if (frame.region !== region) return;
    const now = performance.now();
    const r = frame.blob.kind === 'snapshot' ? world.applySnapshot(frame.blob.snapshot, now) : world.applyDiff(frame.blob.diff, now);
    if (!r.ok && world.needsResync) void session.resync().catch(() => {});
  };
  const ready = await session.open(
    onFrame,
    (msg) => {
      if (msg.op === 'region' || msg.op === 'reconnected') {
        const m = msg as { region: string };
        region = m.region;
        world = new WorldState();
      }
    },
    () => {},
  );
  region = ready.region;

  // 10 Hz is the bridge's visible rate; the 3D loop interpolates in between.
  timer = setInterval(() => {
    const now = performance.now();
    const out: ServerAgent[] = [];
    for (const a of world.list()) {
      const p = sample(a.track, now);
      const before = prev.get(a.id);
      let yaw = before?.yaw ?? 0;
      if (before && (Math.abs(p.x - before.x) > 1e-3 || Math.abs(p.y - before.y) > 1e-3)) {
        yaw = Math.atan2(p.x - before.x, p.y - before.y);
      }
      prev.set(a.id, { x: p.x, y: p.y, yaw });
      out.push({
        id: String(a.id),
        x: p.x,
        y: p.y,
        z: p.z,
        yaw,
        moving: a.track.fromX !== a.track.toX || a.track.fromY !== a.track.toY,
      });
    }
    onAgents(out);
  }, 100);

  return () => {
    if (timer) clearInterval(timer);
    timer = null;
    void session.close().catch(() => {});
    session.dispose();
  };
}
