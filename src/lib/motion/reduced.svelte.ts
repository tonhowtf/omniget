function read(): boolean {
  if (typeof window === "undefined") return false;
  const media = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false;
  const app = document.documentElement.getAttribute("data-reduce-motion") === "true";
  return media || app;
}

export function reducedMotion() {
  let value = $state(read());
  $effect(() => {
    const mq = window.matchMedia?.("(prefers-reduced-motion: reduce)");
    const update = () => (value = read());
    mq?.addEventListener("change", update);
    const observer = new MutationObserver(update);
    observer.observe(document.documentElement, { attributes: true, attributeFilter: ["data-reduce-motion"] });
    return () => {
      mq?.removeEventListener("change", update);
      observer.disconnect();
    };
  });
  return {
    get current() {
      return value;
    },
  };
}

export function loop(tick: (now: number) => boolean | void): () => void {
  let id = 0;
  let stopped = false;
  const frame = (now: number) => {
    if (stopped) return;
    if (tick(now) === false) return;
    id = requestAnimationFrame(frame);
  };
  id = requestAnimationFrame(frame);
  return () => {
    stopped = true;
    cancelAnimationFrame(id);
  };
}
