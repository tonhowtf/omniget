import { describe, expect, it } from 'vitest';
import { fold, fuzzy, highlightRuns, splitQuery } from './fuzzy';

describe('batuta fuzzy', () => {
  it('ignores case and accents without shifting indices', () => {
    expect(fold('Ação Café')).toBe('acao cafe');
    expect(fold('Ação').length).toBe('Ação'.length);
  });

  it('needs every word, somewhere', () => {
    const fields = [
      { text: 'Claude API', weight: 3 },
      { text: 'agent', weight: 1.5 },
    ];
    expect(fuzzy(splitQuery('claude api'), fields)).not.toBeNull();
    expect(fuzzy(splitQuery('claude agent'), fields)).not.toBeNull();
    expect(fuzzy(splitQuery('claude zeta'), fields)).toBeNull();
  });

  it('prefers the name over the body and substrings over scattered letters', () => {
    const name = fuzzy(['plan'], [{ text: 'Plan', weight: 3 }, { text: '', weight: 1 }])!;
    const body = fuzzy(['plan'], [{ text: 'Notes', weight: 3 }, { text: 'the plan', weight: 1 }])!;
    expect(name.score).toBeGreaterThan(body.score);
    const tight = fuzzy(['term'], [{ text: 'Terminal', weight: 3 }])!;
    const loose = fuzzy(['term'], [{ text: 'The error mode', weight: 3 }])!;
    expect(tight.score).toBeGreaterThan(loose.score);
  });

  it('marks matched name characters', () => {
    const r = fuzzy(['cod'], [{ text: 'Codex', weight: 3 }])!;
    expect(highlightRuns('Codex', r.nameIdx)).toEqual([
      { s: 'Cod', hit: true },
      { s: 'ex', hit: false },
    ]);
  });

  it('only takes substrings from long bodies and rejects scattered names', () => {
    const prompt = 'You chase bugs across files and explain each fix';
    expect(fuzzy(['cafe'], [{ text: 'Bug Whisperer', weight: 3 }, { text: prompt, weight: 1, loose: false }])).toBeNull();
    expect(fuzzy(['bugs'], [{ text: 'Bug Whisperer', weight: 3 }, { text: prompt, weight: 1, loose: false }])).not.toBeNull();
    expect(fuzzy(['index'], [{ text: 'Implementation Designer Extra', weight: 3 }])).toBeNull();
  });
});
