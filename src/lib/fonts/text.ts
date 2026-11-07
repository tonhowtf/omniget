export function pluralKey(key: string, count: number): string {
  return count === 1 ? `${key}_one` : key;
}
