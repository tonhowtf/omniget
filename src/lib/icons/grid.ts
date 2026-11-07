export type Range = { first: number; last: number };

export function columnsFor(width: number, cell: number, gap: number): number {
  if (width <= 0) return 1;
  return Math.max(1, Math.floor((width + gap) / (cell + gap)));
}

export function rowRange(scrollTop: number, viewport: number, rowHeight: number, rows: number, overscan = 2): Range {
  if (rows <= 0 || rowHeight <= 0) return { first: 0, last: -1 };
  const first = Math.max(0, Math.floor(scrollTop / rowHeight) - overscan);
  const last = Math.min(rows - 1, Math.ceil((scrollTop + Math.max(viewport, 0)) / rowHeight) + overscan);
  return { first, last };
}

export function moveIndex(index: number, key: string, cols: number, count: number, pageRows = 4): number {
  if (count <= 0) return -1;
  const i = Math.min(Math.max(index, 0), count - 1);
  switch (key) {
    case "ArrowRight":
      return Math.min(i + 1, count - 1);
    case "ArrowLeft":
      return Math.max(i - 1, 0);
    case "ArrowDown":
      return i + cols < count ? i + cols : i;
    case "ArrowUp":
      return i - cols >= 0 ? i - cols : i;
    case "PageDown":
      return Math.min(i + cols * pageRows, count - 1);
    case "PageUp":
      return Math.max(i - cols * pageRows, 0);
    case "Home":
      return 0;
    case "End":
      return count - 1;
    default:
      return i;
  }
}

export function scrollToReveal(index: number, cols: number, rowHeight: number, scrollTop: number, viewport: number): number {
  const row = Math.floor(index / Math.max(cols, 1));
  const top = row * rowHeight;
  const bottom = top + rowHeight;
  if (top < scrollTop) return top;
  if (bottom > scrollTop + viewport) return Math.max(0, bottom - viewport);
  return scrollTop;
}
