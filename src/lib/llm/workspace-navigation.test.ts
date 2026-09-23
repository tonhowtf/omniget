import { expect, it } from 'vitest';
import { LLM_DESTINATIONS, llmDestination } from './workspace-navigation';
import { CORE_NAV_ITEMS } from '../nav-config';
it('keeps every route in exactly one of the seven destinations, Threads first and Chat on /llm', () => {
  expect(LLM_DESTINATIONS).toHaveLength(7);
  expect(LLM_DESTINATIONS[0].href).toBe('/llm/threads');
  expect(LLM_DESTINATIONS.find(d => d.id === 'chat')?.href).toBe('/llm');
  const routes = LLM_DESTINATIONS.flatMap(d => [...d.routes]);
  expect(new Set(routes).size).toBe(routes.length);
  for (const path of routes) expect(llmDestination(`${path}/`).routes).toContain(path);
});
it('places Help immediately after LLM (landing on Threads) in primary navigation', () => {
  expect(llmDestination('/llm/catalog/cct:agents/x').id).toBe('catalog');
  const primary = CORE_NAV_ITEMS.filter(i => i.group === 'primary').sort((a,b) => a.order! - b.order!);
  expect(primary[primary.findIndex(i => i.href === '/llm/threads') + 1].href).toBe('/help');
});
