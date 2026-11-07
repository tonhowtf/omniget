export interface Searchable {
  title: string;
  description: string;
  keywords: string;
}

function fold(text: string): string {
  return text.normalize("NFD").replace(/\p{M}/gu, "").toLocaleLowerCase();
}

function rank(title: string, phrase: string, words: string[]): number {
  if (title.startsWith(phrase)) return 0;
  if (words.every((word) => title.includes(word))) return 1;
  return 2;
}

export function searchPowers<T extends Searchable>(items: T[], query: string): T[] {
  const phrase = fold(query).trim();
  if (!phrase) return items;
  const words = phrase.split(/\s+/);
  return items
    .map((item) => ({ item, title: fold(item.title), text: fold(`${item.title} ${item.keywords} ${item.description}`) }))
    .filter(({ text }) => words.every((word) => text.includes(word)))
    .map(({ item, title }) => ({ item, score: rank(title, phrase, words) }))
    .sort((a, b) => a.score - b.score)
    .map(({ item }) => item);
}
