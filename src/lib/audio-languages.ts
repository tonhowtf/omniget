export type AudioLanguage = { code: string; original: boolean };

type AudioFormat = {
  language?: string | null;
  format_note: string | null;
  has_audio: boolean;
};

const LANGUAGE_CODE = /^[A-Za-z0-9-]{2,12}$/;

export function audioLanguages(formats: AudioFormat[]): AudioLanguage[] {
  const found = new Map<string, AudioLanguage>();
  for (const format of formats) {
    const code = format.language;
    if (!format.has_audio || !code || !LANGUAGE_CODE.test(code)) continue;
    const original = (format.format_note ?? "").toLowerCase().includes("original");
    const known = found.get(code);
    if (known) known.original ||= original;
    else found.set(code, { code, original });
  }
  return [...found.values()].sort((a, b) => Number(b.original) - Number(a.original));
}

export function pickedAudioLanguage(list: AudioLanguage[], code: string): string | null {
  const picked = list.find((language) => language.code === code);
  return picked && !picked.original ? picked.code : null;
}
