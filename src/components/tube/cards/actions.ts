// Svelte actions used by the cards: one shared IntersectionObserver for
// "is this on screen" and a portal that floats menus above scrollers.

type VisibleCb = (visible: boolean) => void;
const callbacks = new WeakMap<Element, VisibleCb>();
let io: IntersectionObserver | null = null;

function observer(): IntersectionObserver {
  io ??= new IntersectionObserver(
    (entries) => {
      for (const e of entries) callbacks.get(e.target)?.(e.isIntersecting);
    },
    { rootMargin: "200px 0px" },
  );
  return io;
}

/** `use:inview={(v) => ...}`: called whenever the element enters/leaves the screen (+200px). */
export function inview(node: Element, cb: VisibleCb) {
  callbacks.set(node, cb);
  observer().observe(node);
  return {
    update(next: VisibleCb) {
      callbacks.set(node, next);
    },
    destroy() {
      callbacks.delete(node);
      io?.unobserve(node);
    },
  };
}

type FloatOpts = {
  /** Element the popup hangs from; null centers it (dialogs). */
  anchor: HTMLElement | null;
  align?: "start" | "end";
  gap?: number;
  /** Prefer opening above the anchor (player-side menus). */
  above?: boolean;
  /** Open at a pointer position instead (right-click menus). */
  point?: { x: number; y: number } | null;
  /** Cover the whole viewport (scrims). */
  fill?: boolean;
};

/**
 * Move the node to the `.omnitube` root (so it keeps the tokens but escapes
 * overflow clipping) and pin it next to the anchor with `position: fixed`.
 * The root may or may not be the containing block for fixed elements
 * (container queries imply layout containment in some engines), so the
 * offset is measured and corrected instead of assumed.
 */
export function floating(node: HTMLElement, opts: FloatOpts) {
  let o = opts;
  const host = (o.anchor?.closest(".omnitube") as HTMLElement | null) ?? (document.querySelector(".omnitube") as HTMLElement | null) ?? document.body;
  host.appendChild(node);
  node.style.position = "fixed";
  node.style.zIndex = "2000";

  function place() {
    node.style.left = "0px";
    node.style.top = "0px";
    const origin = node.getBoundingClientRect();
    const w = node.offsetWidth;
    const h = node.offsetHeight;
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    let x: number;
    let y: number;
    if (o.fill) {
      node.style.left = `${Math.round(-origin.left)}px`;
      node.style.top = `${Math.round(-origin.top)}px`;
      return;
    }
    if (o.point) {
      x = o.point.x;
      y = o.point.y + h <= vh - 8 ? o.point.y : o.point.y - h;
    } else if (!o.anchor) {
      x = (vw - w) / 2;
      y = Math.max(16, (vh - h) / 2);
    } else {
      const a = o.anchor.getBoundingClientRect();
      const gap = o.gap ?? 4;
      x = o.align === "start" ? a.left : a.right - w;
      const below = a.bottom + gap;
      const above = a.top - gap - h;
      y = o.above ? (above >= 8 ? above : below) : below + h <= vh - 8 ? below : above;
    }
    x = Math.min(Math.max(8, x), vw - w - 8);
    y = Math.min(Math.max(8, y), Math.max(8, vh - h - 8));
    node.style.left = `${Math.round(x - origin.left)}px`;
    node.style.top = `${Math.round(y - origin.top)}px`;
  }

  place();
  const ro = new ResizeObserver(place);
  ro.observe(node);
  window.addEventListener("resize", place);
  return {
    update(next: FloatOpts) {
      o = next;
      place();
    },
    destroy() {
      ro.disconnect();
      window.removeEventListener("resize", place);
      node.remove();
    },
  };
}
