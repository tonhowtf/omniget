import { expect, it } from 'vitest';
import { LLM_DESTINATIONS, LLM_PRIMARY_DESTINATIONS, LLM_REDIRECTS, LLM_SETTINGS_HREF, llmDestination } from './workspace-navigation';
import { CORE_NAV_ITEMS } from '../nav-config';
it('keeps every route in exactly one of the nine destinations, Threads first, Work second and Chat on /llm', () => {
  expect(LLM_DESTINATIONS).toHaveLength(9);
  expect(LLM_DESTINATIONS[0].href).toBe('/llm/threads');
  expect(LLM_DESTINATIONS[1].href).toBe('/llm/work');
  expect(LLM_DESTINATIONS.find(d => d.id === 'chat')?.href).toBe('/llm');
  const routes = LLM_DESTINATIONS.flatMap(d => [...d.routes]);
  expect(new Set(routes).size).toBe(routes.length);
  for (const path of routes) expect(llmDestination(`${path}/`).routes).toContain(path);
});
it('groups jobs, loops, missions, the arena and summaries under Work, and the secondary pages under More', () => {
  const work = LLM_DESTINATIONS.find(d => d.id === 'work')!;
  expect(work.tools.map(t => t.href)).toEqual(['/llm/work', '/llm/jobs', '/llm/loops', '/llm/missions', '/llm/arena', '/llm/summaries']);
  const more = LLM_DESTINATIONS.find(d => d.id === 'more')!;
  for (const r of ['/llm/retro', '/llm/reading', '/llm/remote', '/llm/sessions/claude/x']) expect(llmDestination(r).id).toBe('more');
  expect(more.routes).not.toContain('/llm/local');
});
it('keeps the old /llm/local address working through a redirect to Models', () => {
  expect(LLM_REDIRECTS['/llm/local']).toBe('/llm/models');
  expect(llmDestination('/llm/local').id).toBe('configure');
  expect(LLM_DESTINATIONS.flatMap(d => d.tools.map(t => t.href as string))).not.toContain('/llm/local');
});
it('points Configure at Settings AI, off the primary tabs, while the managers keep their own URLs', () => {
  const configure = LLM_DESTINATIONS.find(d => d.id === 'configure')!;
  expect(configure.href).toBe(LLM_SETTINGS_HREF);
  expect(LLM_SETTINGS_HREF).toBe('/settings?tab=ai');
  expect(LLM_PRIMARY_DESTINATIONS.map(d => d.id)).not.toContain('configure');
  const toolHrefs = LLM_DESTINATIONS.flatMap(d => d.tools.map(tool => tool.href as string));
  for (const manager of ['/llm/accounts', '/llm/models', '/llm/mcp', '/llm/skills', '/llm/summaries']) expect(toolHrefs).toContain(manager);
});
it('names Tasks, Routines and Usage and performance with one key each', () => {
  const keys = Object.fromEntries(LLM_DESTINATIONS.flatMap(d => d.tools.map(tool => [tool.href, tool.key])));
  expect(keys['/llm/jobs']).toBe('settings.rebrand.llm.tasks');
  expect(keys['/llm/loops']).toBe('settings.rebrand.llm.routines');
  expect(keys['/llm/observatory']).toBe('settings.rebrand.llm.usage_performance');
});
it('places Help immediately after LLM (landing on Threads) in primary navigation', () => {
  expect(llmDestination('/llm/catalog/cct:agents/x').id).toBe('catalog');
  expect(llmDestination('/llm/company/abc/team').id).toBe('company');
  const primary = CORE_NAV_ITEMS.filter(i => i.group === 'primary').sort((a,b) => a.order! - b.order!);
  expect(primary[primary.findIndex(i => i.href === '/llm/threads') + 1].href).toBe('/help');
});
