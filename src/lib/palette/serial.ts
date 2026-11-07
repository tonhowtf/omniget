export function serialRunner(task: () => Promise<void>): () => Promise<void> {
  let running = false;
  let again = false;
  let pending: Promise<void> = Promise.resolve();
  async function drain() {
    try {
      do {
        again = false;
        await task();
      } while (again);
    } finally {
      running = false;
    }
  }
  return () => {
    if (running) {
      again = true;
      return pending;
    }
    running = true;
    pending = drain();
    return pending;
  };
}
