export function radioStep(key: string, index: number, length: number): number | null {
  if (length <= 0) return null;
  const from = index < 0 ? 0 : index;
  switch (key) {
    case "ArrowRight":
    case "ArrowDown":
      return (from + 1) % length;
    case "ArrowLeft":
    case "ArrowUp":
      return (from - 1 + length) % length;
    case "Home":
      return 0;
    case "End":
      return length - 1;
    default:
      return null;
  }
}

export function rovingIndex<T>(values: readonly T[], current: T): number {
  return Math.max(0, values.indexOf(current));
}

export function radioKeydown<T>(e: KeyboardEvent, values: readonly T[], current: T, pick: (value: T) => void): void {
  const next = radioStep(e.key, values.indexOf(current), values.length);
  if (next === null) return;
  e.preventDefault();
  pick(values[next]);
  const group = (e.currentTarget as HTMLElement | null)?.closest('[role="radiogroup"]');
  group?.querySelectorAll<HTMLElement>('[role="radio"]')[next]?.focus();
}
