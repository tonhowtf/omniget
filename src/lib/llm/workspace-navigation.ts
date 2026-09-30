/**
 * Legacy URLs remain stable; only the active destination exposes its tools.
 * Chat, Agents and Activity are the three primary groups. Configure is not a
 * tab: it links to Settings (AI and agents), while the existing managers keep
 * their URLs and show their own tools when opened.
 */
export const LLM_SETTINGS_HREF = '/settings?tab=ai';

export const LLM_DESTINATIONS = [
  { id: 'chat', href: '/llm', primary: true, routes: ['/llm'], tools: [] },
  { id: 'agents', href: '/llm/roster', primary: true, routes: ['/llm/roster', '/llm/memory', '/llm/reading', '/llm/skills'], tools: [
    { href: '/llm/roster', key: 'llm.workspace.agents' }, { href: '/llm/memory', key: 'assist.memory.nav' }, { href: '/llm/reading', key: 'assist.reading.nav' }, { href: '/llm/skills', key: 'llm.tab.skills' },
  ] },
  { id: 'activity', href: '/llm/missions', primary: true, routes: ['/llm/missions', '/llm/jobs', '/llm/loops', '/llm/observatory', '/llm/summaries'], tools: [
    { href: '/llm/missions', key: 'mission.nav' }, { href: '/llm/jobs', key: 'settings.rebrand.llm.tasks' }, { href: '/llm/loops', key: 'settings.rebrand.llm.routines' }, { href: '/llm/observatory', key: 'settings.rebrand.llm.usage_performance' }, { href: '/llm/summaries', key: 'settings.ai.summarize_title' },
  ] },
  { id: 'configure', href: LLM_SETTINGS_HREF, primary: false, routes: ['/llm/accounts', '/llm/models', '/llm/mcp', '/llm/local', '/llm/arena'], tools: [
    { href: '/llm/accounts', key: 'llm.tab.accounts' }, { href: '/llm/models', key: 'llm.tab.models' }, { href: '/llm/mcp', key: 'llm.tab.mcp' },
  ] },
] as const;

export const LLM_PRIMARY_DESTINATIONS = LLM_DESTINATIONS.filter(d => d.primary);

export function llmDestination(path: string) {
  const normalized = path.replace(/\/$/, '') || '/llm';
  return LLM_DESTINATIONS.find(d => (d.routes as readonly string[]).includes(normalized)) ?? LLM_DESTINATIONS[0];
}
