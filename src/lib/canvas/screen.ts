// L2 positioning without re-rendering: the engine writes --cx, --cy and --z on
// the canvas root once per frame; screen-layer elements compute their place
// with CSS calc from their world coordinates. Text inside stays at its real
// font size, so it is always crisp (02-motor-do-canvas §2.2, L2).

/** `transform` placing the element's top-left at world (x, y). */
export function atWorld(x: number, y: number, dx = 0, dy = 0): string {
  return `translate3d(calc((${x} - var(--cx)) * var(--z) * 1px + ${dx}px), calc((${y} - var(--cy)) * var(--z) * 1px + ${dy}px), 0)`;
}

/** A world length in screen pixels, as a CSS length. */
export function worldLen(v: number, extra = 0): string {
  return `calc(${v} * var(--z) * 1px + ${extra}px)`;
}
