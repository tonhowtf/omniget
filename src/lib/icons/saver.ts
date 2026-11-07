export type SaverHooks<T> = {
  write: (value: T) => Promise<T>;
  applied: (value: T) => void;
  failed: (error: unknown) => void;
};

export type Saver<T> = { save: (value: T) => Promise<boolean>; idle: () => Promise<boolean> };

export function createSaver<T>(hooks: SaverHooks<T>): Saver<T> {
  let pending: { value: T } | null = null;
  let running: Promise<boolean> | null = null;

  async function drain(): Promise<boolean> {
    let outcome: { ok: true; value: T } | { ok: false; error: unknown } | null = null;
    while (pending) {
      const { value } = pending;
      pending = null;
      try {
        outcome = { ok: true, value: await hooks.write(value) };
      } catch (error) {
        outcome = { ok: false, error };
      }
    }
    running = null;
    if (!outcome) return true;
    if (outcome.ok) hooks.applied(outcome.value);
    else hooks.failed(outcome.error);
    return outcome.ok;
  }

  return {
    save(value: T) {
      pending = { value };
      if (!running) running = drain();
      return running;
    },
    idle() {
      return running ?? Promise.resolve(true);
    },
  };
}
