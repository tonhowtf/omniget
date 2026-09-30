import { expect, it } from 'vitest';
import { LLM_DESTINATIONS, LLM_PRIMARY_DESTINATIONS, LLM_SETTINGS_HREF, llmDestination } from './workspace-navigation';
import { CORE_NAV_ITEMS } from '../nav-config';

const LEGACY = ['/llm', '/llm/roster', '/llm/memory', '/llm/reading', '/llm/missions', '/llm/jobs', '/llm/loops', '/llm/observatory',
  '/llm/accounts', '/llm/models', '/llm/mcp', '/llm/skills', '/llm/local', '/llm/arena'];

it('shows Chat, Agents and Activity as the only primary groups', () => {
  expect(LLM_PRIMARY_DESTINATIONS.map(d => d.id)).toEqual(['chat', 'agents', 'activity']);
});

it('keeps every legacy route plus summaries reachable in exactly one destination', () => {
  const routes = LLM_DESTINATIONS.flatMap(d => [...d.routes]);
  expect(new Set(routes).size).toBe(routes.length);
  expect([...routes].sort()).toEqual([...LEGACY, '/llm/summaries'].sort());
  for (const path of routes) expect(llmDestination(`${path}/`).routes).toContain(path);
});

it('points Configure at Settings AI while the managers keep their own URLs', () => {
  const configure = LLM_DESTINATIONS.find(d => d.id === 'configure')!;
  expect(configure.href).toBe(LLM_SETTINGS_HREF);
  expect(LLM_SETTINGS_HREF).toBe('/settings?tab=ai');
  const toolHrefs = LLM_DESTINATIONS.flatMap(d => d.tools.map(tool => tool.href));
  for (const manager of ['/llm/accounts', '/llm/models', '/llm/mcp', '/llm/skills', '/llm/summaries']) expect(toolHrefs).toContain(manager);
});

it('names Tasks, Routines and Usage and performance with one key each', () => {
  const activity = LLM_DESTINATIONS.find(d => d.id === 'activity')!;
  const keys = Object.fromEntries(activity.tools.map(tool => [tool.href, tool.key]));
  expect(keys['/llm/jobs']).toBe('settings.rebrand.llm.tasks');
  expect(keys['/llm/loops']).toBe('settings.rebrand.llm.routines');
  expect(keys['/llm/observatory']).toBe('settings.rebrand.llm.usage_performance');
});

it('places Help immediately after LLM in primary navigation', () => {
  const primary = CORE_NAV_ITEMS.filter(i => i.group === 'primary').sort((a,b) => a.order! - b.order!);
  expect(primary[primary.findIndex(i => i.href === '/llm') + 1].href).toBe('/help');
});
