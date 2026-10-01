export function atWorld(x: number, y: number, dx = 0, dy = 0): string {
  return `translate3d(calc((${x} - var(--cx)) * var(--z) * 1px + ${dx}px), calc((${y} - var(--cy)) * var(--z) * 1px + ${dy}px), 0)`;
}

export function worldLen(v: number, extra = 0): string {
  return `calc(${v} * var(--z) * 1px + ${extra}px)`;
}
