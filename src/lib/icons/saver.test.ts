import { describe, expect, it } from "vitest";
import { createSaver } from "./saver";

function deferred<T>() {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

describe("createSaver", () => {
  it("runs one write at a time and sends only the latest snapshot", async () => {
    const calls: { value: number; done: ReturnType<typeof deferred<number>> }[] = [];
    const applied: number[] = [];
    const saver = createSaver<number>({
      write: (value) => {
        const done = deferred<number>();
        calls.push({ value, done });
        return done.promise;
      },
      applied: (v) => applied.push(v),
      failed: () => {},
    });
    const first = saver.save(1);
    void saver.save(2);
    void saver.save(3);
    expect(calls.map((c) => c.value)).toEqual([1]);
    calls[0].done.resolve(1);
    await Promise.resolve();
    await Promise.resolve();
    expect(calls.map((c) => c.value)).toEqual([1, 3]);
    calls[1].done.resolve(3);
    expect(await first).toBe(true);
    expect(applied).toEqual([3]);
  });

  it("reports failure only when the last write fails", async () => {
    const failures: unknown[] = [];
    const applied: number[] = [];
    let n = 0;
    const saver = createSaver<number>({
      write: async (v) => {
        n += 1;
        if (n === 1) throw new Error("disk");
        return v;
      },
      applied: (v) => applied.push(v),
      failed: (e) => failures.push(e),
    });
    const a = saver.save(1);
    void saver.save(2);
    expect(await a).toBe(true);
    expect(applied).toEqual([2]);
    expect(failures).toEqual([]);
    expect(await saver.save(3)).toBe(true);
    const bad = createSaver<number>({ write: async () => Promise.reject(new Error("x")), applied: () => {}, failed: (e) => failures.push(e) });
    expect(await bad.save(1)).toBe(false);
    expect(failures).toHaveLength(1);
  });
});
