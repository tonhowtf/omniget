// Atalhos da Central: tabela com condição `when` (porte ajustado da tabela
// padrão do T3 Code, estudo 76/02 §6.7), gramática de tecla e de `when`,
// casamento com o evento de teclado e rótulos (glifos no macOS).
//
// Regra = { key, command, when? }. Resolução do fim para o começo (a última
// regra vence); regras do usuário para um comando substituem TODAS as padrão
// daquele comando. `mod` = ⌘ no macOS, Ctrl no resto.

export type WhenContext = {
  /** Foco num campo de texto (input, textarea, select, contenteditable). */
  editableFocus: boolean;
  /** Foco dentro de um terminal xterm. */
  terminalFocus: boolean;
  /** Há uma thread aberta. */
  threadOpen: boolean;
  /** O painel direito está aberto. */
  panelOpen: boolean;
  /** Um turno está rodando na thread aberta. */
  running: boolean;
  /** Há aprovação pendente na thread aberta. */
  approvalPending: boolean;
  /** Há pergunta (user input) pendente na thread aberta. */
  questionPending: boolean;
  paletteOpen: boolean;
  helpOpen: boolean;
  /** Algum outro diálogo modal aberto. */
  modalOpen: boolean;
  isMac: boolean;
  [extra: string]: boolean;
};

export type ShortcutGroup = "threads" | "panel" | "composer" | "approvals" | "navigation" | "terminal" | "app";

export type ShortcutRule = {
  key: string;
  command: string;
  when?: string;
};

export type CommandInfo = {
  id: string;
  group: ShortcutGroup;
  /** Tratado pelo próprio componente (composer, gaveta de aprovação,
   *  terminal): a tabela documenta; o provider só dispara se receber handler. */
  native?: boolean;
};

// ── Comandos ─────────────────────────────────────────────────────────────

export const COMMANDS: CommandInfo[] = [
  { id: "thread.new", group: "threads" },
  { id: "thread.newInProject", group: "threads" },
  { id: "thread.previous", group: "threads" },
  { id: "thread.next", group: "threads" },
  ...Array.from({ length: 9 }, (_, i) => ({ id: `thread.jump.${i + 1}`, group: "threads" as const })),
  { id: "thread.archive", group: "threads" },
  { id: "thread.pin", group: "threads" },
  { id: "thread.snooze", group: "threads" },
  { id: "thread.undo", group: "threads" },
  { id: "thread.rename", group: "threads" },
  { id: "thread.copyId", group: "threads" },
  { id: "thread.fork", group: "threads" },
  { id: "project.add", group: "threads" },
  { id: "sidebar.toggle", group: "threads" },
  { id: "panel.toggle", group: "panel" },
  { id: "panel.close", group: "panel" },
  { id: "panel.diff", group: "panel" },
  { id: "panel.terminal", group: "panel" },
  { id: "panel.files", group: "panel" },
  { id: "panel.plan", group: "panel" },
  { id: "panel.agents", group: "panel" },
  { id: "panel.pr", group: "panel" },
  { id: "composer.focus", group: "composer" },
  { id: "composer.model", group: "composer" },
  { id: "composer.mode", group: "composer" },
  { id: "composer.access", group: "composer" },
  { id: "composer.stash", group: "composer" },
  { id: "composer.send", group: "composer", native: true },
  { id: "composer.sendNow", group: "composer", native: true },
  { id: "composer.newline", group: "composer", native: true },
  { id: "turn.interrupt", group: "composer" },
  { id: "turn.previous", group: "navigation" },
  { id: "turn.next", group: "navigation" },
  { id: "approval.approve", group: "approvals", native: true },
  { id: "approval.approveSession", group: "approvals", native: true },
  { id: "approval.approveAlways", group: "approvals", native: true },
  { id: "approval.deny", group: "approvals", native: true },
  { id: "approval.previous", group: "approvals", native: true },
  { id: "approval.next", group: "approvals", native: true },
  { id: "question.answer", group: "approvals", native: true },
  { id: "palette.toggle", group: "app" },
  { id: "help.toggle", group: "app" },
  { id: "nav.catalog", group: "navigation" },
  { id: "nav.tools", group: "navigation" },
  { id: "nav.sessions", group: "navigation" },
  { id: "nav.settings", group: "navigation" },
  { id: "terminal.clear", group: "terminal", native: true },
  { id: "terminal.close", group: "terminal" },
];

const NOT_TYPING = "!editableFocus && !terminalFocus";
const THREAD_KEYS = "threadOpen && !terminalFocus && !paletteOpen";

export const DEFAULT_RULES: ShortcutRule[] = [
  // Threads
  { key: "mod+n", command: "thread.new", when: "!terminalFocus" },
  { key: "mod+shift+n", command: "thread.newInProject", when: "!terminalFocus" },
  { key: "mod+shift+[", command: "thread.previous" },
  { key: "mod+shift+]", command: "thread.next" },
  { key: "alt+up", command: "thread.previous", when: "!terminalFocus && !paletteOpen" },
  { key: "alt+down", command: "thread.next", when: "!terminalFocus && !paletteOpen" },
  ...Array.from({ length: 9 }, (_, i) => ({ key: `mod+${i + 1}`, command: `thread.jump.${i + 1}`, when: "!terminalFocus" })),
  { key: "mod+backspace", command: "thread.archive", when: `threadOpen && ${NOT_TYPING}` },
  { key: "mod+shift+p", command: "thread.pin", when: THREAD_KEYS },
  { key: "mod+shift+s", command: "thread.snooze", when: THREAD_KEYS },
  { key: "mod+z", command: "thread.undo", when: NOT_TYPING },
  { key: "f2", command: "thread.rename", when: `threadOpen && ${NOT_TYPING}` },
  { key: "mod+shift+c", command: "thread.copyId", when: THREAD_KEYS },
  { key: "mod+shift+k", command: "thread.fork", when: `${THREAD_KEYS} && !running` },
  { key: "mod+alt+n", command: "project.add", when: "!terminalFocus" },
  { key: "mod+b", command: "sidebar.toggle", when: "!terminalFocus" },
  // Painel direito (letras do lançador com ⌥; letras puras seguem no lançador)
  { key: "mod+j", command: "panel.toggle", when: "threadOpen" },
  { key: "mod+w", command: "panel.close", when: "panelOpen && !terminalFocus" },
  { key: "mod+d", command: "panel.diff", when: THREAD_KEYS },
  { key: "alt+d", command: "panel.diff", when: THREAD_KEYS },
  { key: "alt+t", command: "panel.terminal", when: THREAD_KEYS },
  { key: "ctrl+`", command: "panel.terminal", when: "threadOpen" },
  { key: "alt+f", command: "panel.files", when: THREAD_KEYS },
  { key: "alt+p", command: "panel.plan", when: THREAD_KEYS },
  { key: "alt+a", command: "panel.agents", when: THREAD_KEYS },
  { key: "alt+r", command: "panel.pr", when: THREAD_KEYS },
  // Composer
  { key: "mod+l", command: "composer.focus", when: "threadOpen && !paletteOpen" },
  { key: "mod+shift+m", command: "composer.model", when: THREAD_KEYS },
  { key: "mod+shift+a", command: "composer.mode", when: THREAD_KEYS },
  { key: "mod+shift+x", command: "composer.access", when: THREAD_KEYS },
  { key: "mod+s", command: "composer.stash", when: THREAD_KEYS },
  { key: "enter", command: "composer.send", when: "editableFocus && !running" },
  { key: "mod+enter", command: "composer.sendNow", when: "editableFocus && running" },
  { key: "shift+enter", command: "composer.newline", when: "editableFocus" },
  { key: "mod+.", command: "turn.interrupt", when: "running && !terminalFocus" },
  { key: "escape", command: "turn.interrupt", when: `running && ${NOT_TYPING} && !paletteOpen && !helpOpen && !modalOpen` },
  { key: "mod+alt+up", command: "turn.previous", when: `threadOpen && ${NOT_TYPING}` },
  { key: "mod+alt+down", command: "turn.next", when: `threadOpen && ${NOT_TYPING}` },
  // Aprovações e perguntas (fora de campo de texto)
  { key: "y", command: "approval.approve", when: `approvalPending && ${NOT_TYPING}` },
  { key: "s", command: "approval.approveSession", when: `approvalPending && ${NOT_TYPING}` },
  { key: "a", command: "approval.approveAlways", when: `approvalPending && ${NOT_TYPING}` },
  { key: "n", command: "approval.deny", when: `approvalPending && ${NOT_TYPING}` },
  { key: "[", command: "approval.previous", when: `approvalPending && ${NOT_TYPING}` },
  { key: "]", command: "approval.next", when: `approvalPending && ${NOT_TYPING}` },
  { key: "1", command: "question.answer", when: `questionPending && ${NOT_TYPING}` },
  // App
  { key: "mod+k", command: "palette.toggle", when: "!terminalFocus" },
  { key: "?", command: "help.toggle", when: `${NOT_TYPING} && !paletteOpen` },
  { key: "mod+/", command: "help.toggle", when: "!terminalFocus" },
  { key: "mod+shift+t", command: "nav.tools", when: "!terminalFocus" },
  { key: "mod+shift+o", command: "nav.catalog", when: "!terminalFocus" },
  { key: "mod+shift+h", command: "nav.sessions", when: "!terminalFocus" },
  { key: "mod+,", command: "nav.settings" },
  // Terminal
  { key: "mod+k", command: "terminal.clear", when: "terminalFocus" },
  { key: "mod+w", command: "terminal.close", when: "terminalFocus" },
];

export function commandInfo(id: string): CommandInfo | undefined {
  return COMMANDS.find((c) => c.id === id);
}

/** Chave i18n do nome de um comando (`thread.jump.3` → `…cmd.thread_jump`). */
export function commandLabelKey(id: string): string {
  const base = id.startsWith("thread.jump.") ? "thread.jump" : id;
  return `llm.central.shortcuts.cmd.${base.replace(/\./g, "_")}`;
}

/** Regras do usuário substituem todas as padrão do mesmo comando (máx. 256). */
export function mergeRules(defaults: ShortcutRule[], user: ShortcutRule[]): ShortcutRule[] {
  const overridden = new Set(user.map((r) => r.command));
  return [...defaults.filter((r) => !overridden.has(r.command)), ...user].slice(0, 256);
}

const USER_KEY = "omniget.central.shortcuts.v1";

/** Regras do usuário guardadas (localStorage), validadas. */
export function loadUserRules(): ShortcutRule[] {
  try {
    const v = JSON.parse(localStorage.getItem(USER_KEY) ?? "[]");
    if (!Array.isArray(v)) return [];
    return v.filter(
      (r) =>
        r &&
        typeof r.key === "string" &&
        typeof r.command === "string" &&
        parseKey(r.key) !== null &&
        (r.when === undefined || (typeof r.when === "string" && parseWhen(r.when) !== null)),
    );
  } catch {
    return [];
  }
}

export function saveUserRules(rules: ShortcutRule[]) {
  try {
    localStorage.setItem(USER_KEY, JSON.stringify(rules.slice(0, 256)));
  } catch {
    /* storage desligado */
  }
}

// ── Gramática de tecla ───────────────────────────────────────────────────

export type ParsedKey = {
  key: string;
  mod: boolean;
  ctrl: boolean;
  meta: boolean;
  shift: boolean;
  alt: boolean;
};

const KEY_ALIASES: Record<string, string> = {
  esc: "escape",
  return: "enter",
  space: " ",
  del: "delete",
  arrowup: "up",
  arrowdown: "down",
  arrowleft: "left",
  arrowright: "right",
  plus: "+",
};

/** `mod+shift+[` → ParsedKey; `+` final é a própria tecla mais. */
export function parseKey(spec: string): ParsedKey | null {
  let s = spec.trim().toLowerCase();
  if (!s) return null;
  let plusKey = false;
  if (s === "+" || s.endsWith("++")) {
    plusKey = true;
    s = s === "+" ? "" : s.slice(0, -2);
  }
  const parts = s ? s.split("+") : [];
  const out: ParsedKey = { key: "", mod: false, ctrl: false, meta: false, shift: false, alt: false };
  const keys: string[] = [];
  for (const p of parts) {
    if (!p) return null;
    if (p === "mod") out.mod = true;
    else if (p === "cmd" || p === "meta" || p === "command") out.meta = true;
    else if (p === "ctrl" || p === "control") out.ctrl = true;
    else if (p === "shift") out.shift = true;
    else if (p === "alt" || p === "option" || p === "opt") out.alt = true;
    else keys.push(KEY_ALIASES[p] ?? p);
  }
  if (plusKey) keys.push("+");
  if (keys.length !== 1) return null;
  out.key = keys[0];
  return out;
}

const CODE_KEYS: Record<string, string> = {
  BracketLeft: "[",
  BracketRight: "]",
  Slash: "/",
  Backslash: "\\",
  Period: ".",
  Comma: ",",
  Semicolon: ";",
  Quote: "'",
  Backquote: "`",
  Minus: "-",
  Equal: "=",
  Space: " ",
};

/** Tecla física do evento, independente do layout e do ⌥ do macOS. */
function codeKey(code: string): string | null {
  if (code.startsWith("Key") && code.length === 4) return code.slice(3).toLowerCase();
  if (code.startsWith("Digit") && code.length === 6) return code.slice(5);
  if (code.startsWith("Numpad") && /^Numpad\d$/.test(code)) return code.slice(6);
  return CODE_KEYS[code] ?? null;
}

function eventKey(key: string): string {
  const k = key.toLowerCase();
  return KEY_ALIASES[k] ?? k;
}

export type KeyEventLike = Pick<KeyboardEvent, "key" | "code" | "metaKey" | "ctrlKey" | "shiftKey" | "altKey">;

/** O evento casa com a tecla? Modificadores exatos (salvo `?`, que já implica ⇧). */
export function matchKey(p: ParsedKey, e: KeyEventLike, isMac: boolean): boolean {
  const wantMeta = p.meta || (p.mod && isMac);
  const wantCtrl = p.ctrl || (p.mod && !isMac);
  if (e.metaKey !== wantMeta || e.ctrlKey !== wantCtrl || e.altKey !== p.alt) return false;
  if (p.key === "?") return e.key === "?";
  if (e.shiftKey !== p.shift) return false;
  const k = eventKey(e.key);
  if (k === p.key) return true;
  // ⌥/⇧ trocam o caractere (⌥D = ∂, ⇧[ = {): cai para a tecla física.
  const c = codeKey(e.code);
  return c !== null && c === p.key;
}

// ── Gramática de `when` ──────────────────────────────────────────────────

export type WhenNode =
  | { t: "id"; name: string }
  | { t: "lit"; value: boolean }
  | { t: "not"; a: WhenNode }
  | { t: "and"; a: WhenNode; b: WhenNode }
  | { t: "or"; a: WhenNode; b: WhenNode };

const whenCache = new Map<string, WhenNode | null>();

/** `a && !(b || c)`; identificadores, `!`, `&&`, `||`, parênteses, true/false. */
export function parseWhen(src: string): WhenNode | null {
  if (whenCache.has(src)) return whenCache.get(src)!;
  const result = parseWhenUncached(src);
  whenCache.set(src, result);
  return result;
}

function parseWhenUncached(src: string): WhenNode | null {
  if (src.length > 256) return null;
  const tokens = src.match(/\s*(&&|\|\||!|\(|\)|[A-Za-z_][A-Za-z0-9_.]*|\S)/g)?.map((x) => x.trim()) ?? [];
  let i = 0;
  let depth = 0;
  const peek = () => tokens[i];
  const next = () => tokens[i++];

  function primary(): WhenNode | null {
    const tok = next();
    if (tok === undefined) return null;
    if (tok === "!") {
      const a = primary();
      return a ? { t: "not", a } : null;
    }
    if (tok === "(") {
      if (++depth > 64) return null;
      const e = or();
      if (!e || next() !== ")") return null;
      depth--;
      return e;
    }
    if (tok === "true" || tok === "false") return { t: "lit", value: tok === "true" };
    if (/^[A-Za-z_][A-Za-z0-9_.]*$/.test(tok)) return { t: "id", name: tok };
    return null;
  }
  function and(): WhenNode | null {
    let a = primary();
    while (a && peek() === "&&") {
      next();
      const b = primary();
      if (!b) return null;
      a = { t: "and", a, b };
    }
    return a;
  }
  function or(): WhenNode | null {
    let a = and();
    while (a && peek() === "||") {
      next();
      const b = and();
      if (!b) return null;
      a = { t: "or", a, b };
    }
    return a;
  }
  const root = or();
  return root && i === tokens.length ? root : null;
}

export function evalWhen(node: WhenNode, ctx: Partial<WhenContext>): boolean {
  switch (node.t) {
    case "lit":
      return node.value;
    case "id":
      return !!ctx[node.name];
    case "not":
      return !evalWhen(node.a, ctx);
    case "and":
      return evalWhen(node.a, ctx) && evalWhen(node.b, ctx);
    case "or":
      return evalWhen(node.a, ctx) || evalWhen(node.b, ctx);
  }
}

/** `when` ausente = sempre; `when` inválido = nunca. */
export function whenHolds(when: string | undefined, ctx: Partial<WhenContext>): boolean {
  if (!when || !when.trim()) return true;
  const n = parseWhen(when);
  return n ? evalWhen(n, ctx) : false;
}

// ── Resolução ────────────────────────────────────────────────────────────

/**
 * Comando do evento: percorre do fim para o começo e devolve a primeira regra
 * cuja tecla casa, cujo `when` vale e que `accept` aceita (ex.: tem handler).
 */
export function resolve(
  rules: ShortcutRule[],
  e: KeyEventLike,
  ctx: Partial<WhenContext>,
  accept: (command: string) => boolean = () => true,
): ShortcutRule | null {
  const isMac = !!ctx.isMac;
  for (let i = rules.length - 1; i >= 0; i--) {
    const r = rules[i];
    const p = parseKey(r.key);
    if (!p || !matchKey(p, e, isMac)) continue;
    if (!whenHolds(r.when, ctx)) continue;
    if (!accept(r.command)) continue;
    return r;
  }
  return null;
}

// ── Rótulos ──────────────────────────────────────────────────────────────

const KEY_LABELS: Record<string, [string, string]> = {
  up: ["↑", "Up"],
  down: ["↓", "Down"],
  left: ["←", "Left"],
  right: ["→", "Right"],
  enter: ["↩", "Enter"],
  escape: ["⎋", "Esc"],
  backspace: ["⌫", "Backspace"],
  delete: ["⌦", "Delete"],
  tab: ["⇥", "Tab"],
  " ": ["Space", "Space"],
};

/** Rótulo de uma tecla: `⌃⌥⇧⌘K` no macOS, `Ctrl+Alt+Shift+K` no resto. */
export function keyLabel(spec: string, isMac: boolean): string {
  const p = parseKey(spec);
  if (!p) return spec;
  const k = KEY_LABELS[p.key]?.[isMac ? 0 : 1] ?? (p.key.length === 1 ? p.key.toUpperCase() : p.key.toUpperCase());
  const ctrl = p.ctrl || (p.mod && !isMac);
  const meta = p.meta || (p.mod && isMac);
  if (isMac) return `${ctrl ? "⌃" : ""}${p.alt ? "⌥" : ""}${p.shift ? "⇧" : ""}${meta ? "⌘" : ""}${k}`;
  const parts: string[] = [];
  if (ctrl) parts.push("Ctrl");
  if (meta) parts.push("Win");
  if (p.alt) parts.push("Alt");
  if (p.shift) parts.push("Shift");
  parts.push(k);
  return parts.join("+");
}

/** Teclas de um comando (para chips na paleta e na ajuda). */
export function keysFor(rules: ShortcutRule[], command: string): ShortcutRule[] {
  return rules.filter((r) => r.command === command);
}

export function detectMac(): boolean {
  if (typeof navigator === "undefined") return false;
  const nav = navigator as Navigator & { userAgentData?: { platform?: string } };
  const platform = nav.userAgentData?.platform ?? navigator.platform ?? "";
  return /mac|iphone|ipad/i.test(platform);
}

/** Contexto que dá para ler do DOM (foco, diálogos). */
export function domContext(root: Document = document): Pick<WhenContext, "editableFocus" | "terminalFocus" | "modalOpen"> {
  const el = root.activeElement as HTMLElement | null;
  const terminalFocus = !!el?.closest?.(".xterm");
  const editableFocus =
    !terminalFocus &&
    !!el &&
    (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.tagName === "SELECT" || el.isContentEditable);
  const modalOpen = !!root.querySelector("dialog[open]:not([data-central-overlay])");
  return { editableFocus, terminalFocus, modalOpen };
}
