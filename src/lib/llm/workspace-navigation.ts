export const LLM_SETTINGS_HREF = '/settings?tab=ai';

const LLM_DESTINATIONS = [
  { id: 'threads', href: '/llm/threads', primary: true, routes: ['/llm/threads'], tools: [] },
  { id: 'work', href: '/llm/work', primary: true, routes: ['/llm/work', '/llm/jobs', '/llm/loops', '/llm/missions', '/llm/arena', '/llm/summaries'], tools: [
    { href: '/llm/work', key: 'llm.work.nav' }, { href: '/llm/jobs', key: 'settings.rebrand.llm.tasks' }, { href: '/llm/loops', key: 'settings.rebrand.llm.routines' },
    { href: '/llm/missions', key: 'mission.nav' }, { href: '/llm/arena', key: 'llm.tab.arena' }, { href: '/llm/summaries', key: 'settings.ai.summarize_title' },
  ] },
  { id: 'chat', href: '/llm', primary: true, routes: ['/llm'], tools: [] },
  { id: 'agents', href: '/llm/roster', primary: true, routes: ['/llm/roster', '/llm/memory', '/llm/skills'], tools: [
    { href: '/llm/roster', key: 'llm.workspace.agents' }, { href: '/llm/memory', key: 'assist.memory.nav' }, { href: '/llm/skills', key: 'llm.tab.skills' },
  ] },
  { id: 'company', href: '/llm/company', primary: true, routes: ['/llm/company'], tools: [], label: 'company.nav' },
  { id: 'catalog', href: '/llm/catalog', primary: true, routes: ['/llm/catalog', '/llm/installed', '/llm/catalog/wizard'], tools: [
    { href: '/llm/catalog', key: 'llm.workspace.catalog' }, { href: '/llm/installed', key: 'llm.workspace.installed' }, { href: '/llm/catalog/wizard', key: 'llm.workspace.wizard' },
  ] },
  { id: 'tools', href: '/llm/tools', primary: true, routes: ['/llm/tools'], tools: [] },
  { id: 'configure', href: LLM_SETTINGS_HREF, primary: false, routes: ['/llm/accounts', '/llm/models', '/llm/mcp'], tools: [
    { href: '/llm/accounts', key: 'llm.tab.accounts' }, { href: '/llm/models', key: 'llm.tab.models' }, { href: '/llm/mcp', key: 'llm.tab.mcp' },
  ] },
  { id: 'more', href: '/llm/observatory', primary: true, routes: ['/llm/observatory', '/llm/usage', '/llm/sessions', '/llm/retro', '/llm/reading', '/llm/remote'], tools: [
    { href: '/llm/observatory', key: 'settings.rebrand.llm.usage_performance' }, { href: '/llm/usage', key: 'llm.workspace.usage' }, { href: '/llm/sessions', key: 'llm.workspace.sessions' },
    { href: '/llm/retro', key: 'llm.workspace.retro' }, { href: '/llm/reading', key: 'assist.reading.nav' }, { href: '/llm/remote', key: 'llm.workspace.remote' },
  ] },
] as const;

export const LLM_PRIMARY_DESTINATIONS = LLM_DESTINATIONS.filter(d => d.primary);

export const LLM_REDIRECTS: Record<string, string> = { '/llm/local': '/llm/models' };

export function llmDestination(path: string) {
  const raw = path.replace(/\/$/, '') || '/llm';
  const normalized = LLM_REDIRECTS[raw] ?? raw;
  const exact = LLM_DESTINATIONS.find(d => (d.routes as readonly string[]).includes(normalized));
  if (exact) return exact;
  const nested = LLM_DESTINATIONS.find(d => (d.routes as readonly string[]).some(r => r !== '/llm' && normalized.startsWith(r + '/')));
  return nested ?? LLM_DESTINATIONS[2];
}
export function llmDestinationLabel(d: (typeof LLM_DESTINATIONS)[number]): string {
  return 'label' in d ? d.label : `llm.workspace.${d.id}`;
}
