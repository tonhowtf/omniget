import { describe, expect, it } from 'vitest';
import { resolvePath, smartClean, tokenAt } from './text';

describe('clickable paths', () => {
  const cells = (s: string) => [...s];
  it('takes a path with parentheses and drops sentence punctuation', () => {
    const line = 'see src/(app)/page.tsx, then';
    expect(tokenAt(cells(line), 8)).toBe('src/(app)/page.tsx');
    expect(tokenAt(cells('(open https://a.dev/x).'), 8)).toBe('https://a.dev/x');
    expect(tokenAt(cells('error at lib/a.rs:12:4'), 12)).toBe('lib/a.rs');
    expect(tokenAt(cells('cat "my file.txt" ok'), 9)).toBe('my file.txt');
  });
  it('resolves relative to the shell directory', () => {
    expect(resolvePath('src/a.ts', 'file://host/Users/me/repo')).toBe('/Users/me/repo/src/a.ts');
    expect(resolvePath('../b', '/Users/me/repo')).toBe('/Users/me/b');
    expect(resolvePath('x', '')).toBeNull();
  });
});

describe('smart copy', () => {
  it('joins lines broken at the margin and strips box borders', () => {
    const text = '╭────────╮\n│ hello  │\n╰────────╯\nThis is a long line that the\nterminal wrapped here.\n\n- item one\n- item two';
    expect(smartClean(text, 80)).toBe('hello\nThis is a long line that the terminal wrapped here.\n\n- item one\n- item two');
  });
});
