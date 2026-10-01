import content from './content.json';
type HelpArticle = (typeof content)[number];
const helpLocale = (locale: string) => locale.startsWith('pt') ? 'pt' : locale.startsWith('ru') ? 'ru' : 'en';
export function articles(locale: string): HelpArticle[] { return content.filter(a => a.locale === helpLocale(locale)); }
const normalize = (s: string) => s.normalize('NFD').replace(/[\u0300-\u036f]/g, '').toLowerCase();
export function contentHash(a: HelpArticle): string {
  let h = 2166136261;
  for (const c of new TextEncoder().encode([a.id, a.locale, a.appVersion, a.summary, ...a.steps].join('|'))) h = Math.imul(h ^ c, 16777619) >>> 0;
  return h.toString(16).padStart(8, '0');
}
export function searchDocs(query: string, locale: string, limit = 8) {
  const terms = normalize(query).split(/\s+/).filter(Boolean);
  return articles(locale).map(article => {
    const title = normalize(article.title + ' ' + article.tags);
    const body = normalize(article.summary + ' ' + article.steps.join(' '));
    const score = terms.reduce((n, term) => n + (title.includes(term) ? 5 : body.includes(term) ? 1 : 0), 0);
    return { ...article, articleId: article.id, excerpt: article.summary, contentHash: contentHash(article), score };
  }).filter(a => !terms.length || a.score > 0).sort((a,b) => b.score - a.score || a.id.localeCompare(b.id)).slice(0, Math.max(1, Math.min(20, limit)));
}
export function resolveCitation(value: string, locale: string): string | null {
  const match = /^help:\/\/([a-z-]+)#guide$/.exec(value);
  return match && articles(locale).some(a => a.id === match[1]) ? `/help?article=${match[1]}#guide` : null;
}
export function readArticle(id: string, locale: string) { return articles(locale).find(a => a.id === id); }
