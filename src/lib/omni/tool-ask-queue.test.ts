import { describe, expect, it } from 'vitest';
import { acceptAsk, type PendingAsk } from './tool-ask-queue';

const base = { request_id: 'run-1', tool_call_id: 'call-1', tool: 'Bash', agent: 'Reviewer' };

function one(raw: unknown): PendingAsk {
  const queue = acceptAsk([], raw);
  expect(queue).toHaveLength(1);
  return queue[0];
}

describe('acceptAsk', () => {
  it('keeps a known kind', () => {
    expect(one({ ...base, kind: 'command' }).kind).toBe('command');
    expect(one({ ...base, kind: 'file_change' }).kind).toBe('file_change');
  });

  it('falls back to other when the kind is missing or unknown', () => {
    expect(one(base).kind).toBe('other');
    expect(one({ ...base, kind: 'launch_missiles' }).kind).toBe('other');
    expect(one({ ...base, kind: 7 }).kind).toBe('other');
  });

  it('keeps the asked time and the deadline only when they are real timestamps', () => {
    const ask = one({ ...base, asked_ms: 1_700_000_000_000, deadline_ms: 1_700_000_240_000 });
    expect(ask.askedMs).toBe(1_700_000_000_000);
    expect(ask.deadlineMs).toBe(1_700_000_240_000);
    const bare = one({ ...base, asked_ms: 'soon', deadline_ms: null });
    expect(bare.askedMs).toBeNull();
    expect(bare.deadlineMs).toBeNull();
    expect(one({ ...base, asked_ms: -5, deadline_ms: Number.NaN }).askedMs).toBeNull();
  });

  it('offers always unless the server says it would not stick', () => {
    expect(one(base).canAlways).toBe(true);
    expect(one({ ...base, can_always: false }).canAlways).toBe(false);
  });

  it('strips direction controls that could disguise who is asking', () => {
    expect(one({ ...base, agent: 'Rev\u202Eiewer\u200B' }).agent).toBe('Reviewer');
  });

  it('still refuses a request without ids', () => {
    expect(acceptAsk([], { tool: 'Bash' })).toHaveLength(0);
  });

  it('keeps the measured impact and drops anything that is not a count', () => {
    const ask = one({ ...base, impact: { files: 3, added: 42, removed: 7, outside: true } });
    expect(ask.impact).toEqual({ files: 3, added: 42, removed: 7, outside: true });
    expect(one({ ...base, impact: { files: 'many', added: -1 } }).impact).toBeNull();
    expect(one({ ...base, impact: null }).impact).toBeNull();
    expect(one(base).impact).toBeNull();
  });

  it('shows the canvas agent by its name, not by the node id', () => {
    const ask = one({ ...base, agent: 'canvas:a9192781-c654', tool: 'Write (Tester)' });
    expect(ask.agent).toBe('Tester');
    expect(ask.tool).toBe('Write');
    const plain = one({ ...base, agent: 'reviewer', tool: 'fs_write' });
    expect([plain.agent, plain.tool]).toEqual(['reviewer', 'fs_write']);
    const odd = one({ ...base, agent: 'reviewer', tool: 'thing (x)' });
    expect([odd.agent, odd.tool]).toEqual(['reviewer', 'thing (x)']);
  });
});
