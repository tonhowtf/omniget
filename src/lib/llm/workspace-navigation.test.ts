import { expect, it } from 'vitest';
import { LLM_DESTINATIONS, llmDestination } from './workspace-navigation';
import { CORE_NAV_ITEMS } from '../nav-config';
it('keeps the legacy routes plus memory and reading in exactly four destinations', () => {
  expect(LLM_DESTINATIONS).toHaveLength(4);
  const routes = LLM_DESTINATIONS.flatMap(d => [...d.routes]);
  expect(new Set(routes).size).toBe(14);
  for (const path of routes) expect(llmDestination(`${path}/`).routes).toContain(path);
});
it('places Help immediately after LLM in primary navigation', () => {
  const primary = CORE_NAV_ITEMS.filter(i => i.group === 'primary').sort((a,b) => a.order! - b.order!);
  expect(primary[primary.findIndex(i => i.href === '/llm') + 1].href).toBe('/help');
});
