export function waitForPaint(): Promise<void> {
  return new Promise(resolve => {
    let finished = false;
    let frame = 0;
    const finish = () => {
      if (finished) return;
      finished = true;
      clearTimeout(timer);
      cancelAnimationFrame(frame);
      resolve();
    };
    const timer = setTimeout(finish, 100);
    frame = requestAnimationFrame(() => { frame = requestAnimationFrame(finish); });
  });
}
