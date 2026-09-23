// Busca da paleta de comandos: termos separados por espaço (AND), ranking
// exato > prefixo > substring, termos anteriores pesam mais.

export type PaletteItem = {
  id: string;
  group: "actions" | "threads" | "catalog" | "navigation";
  title: string;
  subtitle?: string;
  /** Rótulo de atalho (chip). */
  shortcut?: string;
  /** Texto extra buscável (id da thread, tags…); não aparece. */
  terms?: string;
  /** Afunda no grupo. */
  secondary?: boolean;
  disabled?: boolean;
  run: () => void | Promise<void>;
};

function norm(s: string): string {
  return s
    .toLowerCase()
    .normalize("NFD")
    .replace(/[̀-ͯ]/g, "");
}

/** Pontuação do item para a consulta (0 = não casa). */
export function score(item: Pick<PaletteItem, "title" | "subtitle" | "terms">, query: string): number {
  const words = norm(query).split(/\s+/).filter(Boolean);
  if (words.length === 0) return 1;
  const title = norm(item.title);
  const rest = norm(`${item.subtitle ?? ""} ${item.terms ?? ""}`);
  let total = 0;
  for (let i = 0; i < words.length; i++) {
    const w = words[i];
    const weight = words.length - i;
    let s = 0;
    if (title === w) s = 100;
    else if (title.startsWith(w)) s = 60;
    else if (title.split(/[\s/._-]+/).some((part) => part.startsWith(w))) s = 40;
    else if (title.includes(w)) s = 25;
    else if (rest.includes(w)) s = 10;
    if (s === 0) return 0;
    total += s * weight;
  }
  return total;
}

/** Filtra e ordena mantendo os grupos na ordem dada. */
export function filterItems(items: PaletteItem[], query: string, groupOrder: PaletteItem["group"][]): PaletteItem[] {
  const scored = items
    .map((it, i) => ({ it, i, s: score(it, query) }))
    .filter((x) => x.s > 0)
    .sort(
      (a, b) =>
        groupOrder.indexOf(a.it.group) - groupOrder.indexOf(b.it.group) ||
        Number(!!a.it.secondary) - Number(!!b.it.secondary) ||
        b.s - a.s ||
        a.i - b.i,
    );
  return scored.map((x) => x.it);
}
