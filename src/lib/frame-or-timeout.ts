export function onFrameOrAfter(run: () => void, maxWaitMs: number): void {
  let done = false;
  let frame = 0;
  const once = () => {
    if (done) return;
    done = true;
    clearTimeout(timer);
    cancelAnimationFrame(frame);
    run();
  };
  const timer = setTimeout(once, maxWaitMs);
  frame = requestAnimationFrame(once);
}
