/**
 * Rótulos de atalho que existem no teclado de quem está lendo.
 *
 * O `⌘` estava fixo na barra de ferramentas. Em Windows e Linux isso é pior que
 * feio: a tecla não existe ali, então a dica deixa de ser dica — o usuário não
 * tem como agir sobre ela, e o app passa a sinalizar que foi feito para outra
 * plataforma. O atalho em si sempre aceitou as duas teclas
 * (`e.metaKey || e.ctrlKey`); só o rótulo mentia.
 */

/** `true` no macOS. */
export function isMac(): boolean {
  if (typeof navigator === "undefined") return false;
  // `userAgentData.platform` é o caminho novo; `navigator.platform` está
  // deprecado mas é o único que responde em WebKit, que é justamente o motor
  // onde o app roda no macOS.
  const uaData = (navigator as Navigator & { userAgentData?: { platform?: string } }).userAgentData;
  const raw = uaData?.platform || navigator.platform || "";
  return /mac/i.test(raw);
}

/** `⌘` no macOS, `Ctrl` no resto. */
export function modKey(): string {
  return isMac() ? "⌘" : "Ctrl";
}

/**
 * Atalho pronto para exibir, com o separador que cada plataforma usa.
 *
 * macOS junta as teclas (`⌘K`); Windows e Linux separam com `+` (`Ctrl+K`).
 * Escrever `⌘+K` ou `CtrlK` é o tipo de detalhe que faz o app parecer
 * traduzido em vez de nativo.
 */
export function shortcut(...keys: string[]): string {
  return isMac() ? `${modKey()}${keys.join("")}` : [modKey(), ...keys].join("+");
}

/**
 * Rótulo de um atalho gravado no formato do Tauri (`CmdOrCtrl+Shift+K`).
 *
 * Vive aqui e não em cada componente porque duas cópias divergiram: uma
 * traduzia `Meta` para `Win` fora do macOS e a outra deixava `Meta` cru, então
 * o mesmo atalho aparecia com nomes diferentes em duas telas do mesmo app.
 */
export function formatBinding(binding: string): string {
  if (!binding) return "";
  const mac = isMac();
  return binding
    .split("+")
    .map((part) => {
      if (part === "CmdOrCtrl") return modKey();
      if (part === "Meta" || part === "Super" || part === "Command") return mac ? "⌘" : "Win";
      if (part === "Alt") return mac ? "⌥" : "Alt";
      if (part === "Shift") return mac ? "⇧" : "Shift";
      if (part === "Control" || part === "Ctrl") return mac ? "⌃" : "Ctrl";
      return part;
    })
    .join(mac ? "" : "+");
}

export type OsName = "windows" | "macos" | "linux";

/** Sistema onde o app está rodando, para avisar quando uma ferramenta não roda nele. */
export function currentOs(): OsName {
  if (isMac()) return "macos";
  if (typeof navigator === "undefined") return "linux";
  const uaData = (navigator as Navigator & { userAgentData?: { platform?: string } }).userAgentData;
  const raw = `${uaData?.platform || ""} ${navigator.platform || ""} ${navigator.userAgent || ""}`;
  return /win/i.test(raw) ? "windows" : "linux";
}
