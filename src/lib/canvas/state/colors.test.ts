import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { STATE_COLORS, stateCss, stateVar, type AgentState } from './colors';

const kebab = (s: string) => s.replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`);
const css = readFileSync('src/app.css', 'utf8');
const states = Object.keys(STATE_COLORS) as AgentState[];

describe('agent state colors', () => {
  it('returns a themed variable with the hex fallback', () => {
    expect(stateVar('waitingPeer')).toBe('var(--agent-waiting-peer, #7f8fc9)');
    expect(stateVar('blocked')).toBe('var(--agent-blocked, #d96a62)');
  });

  it('treats a missing state as unknown', () => {
    expect(stateVar(undefined)).toBe('var(--agent-unknown, #8a8a8a)');
    expect(stateCss(undefined)).toBe('#8a8a8a');
  });

  it('defines one token per state with the same default value', () => {
    for (const s of states) {
      const m = css.match(new RegExp(`--agent-${kebab(s)}:\\s*(#[0-9a-fA-F]{6})`));
      expect(m, `--agent-${kebab(s)} missing in app.css`).not.toBeNull();
      expect(m![1].toLowerCase()).toBe(stateCss(s));
    }
  });

  it('overrides every state in each light theme', () => {
    for (const theme of ['light', 'catppuccin-latte', 'nyxvamp-radiance', 'eink-day', 'eink-sepia']) {
      const start = css.indexOf(`[data-theme="${theme}"] {`);
      expect(start, theme).toBeGreaterThan(-1);
      const block = css.slice(start, css.indexOf('\n}', start));
      for (const s of states) expect(block, `${theme} --agent-${kebab(s)}`).toContain(`--agent-${kebab(s)}:`);
    }
  });
});
