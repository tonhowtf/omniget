<script lang="ts" module>
  import type { WhenContext } from "$lib/central/shortcuts";

  /** Handler de um comando; `arg` é o N de `thread.jump.N`. */
  export type ShortcutHandler = (e: KeyboardEvent | null, arg?: number) => void;

  export type PaletteThread = {
    id: string;
    title: string;
    /** Projeto · status · driver (aparece ao lado). */
    subtitle?: string;
    /** Texto extra buscável (branch, id…). */
    terms?: string;
    archived?: boolean;
  };

  export type PaletteAction = {
    id: string;
    title: string;
    subtitle?: string;
    shortcut?: string;
    run: () => void | Promise<void>;
  };

  export type ShortcutsContext = Partial<Omit<WhenContext, "editableFocus" | "terminalFocus" | "paletteOpen" | "helpOpen" | "isMac">>;
</script>

<script lang="ts">
  /**
   * Handler global de atalhos da Central, montável numa rota (ex.: threads).
   * Escuta `keydown` na fase de captura, monta o contexto `when` (foco em
   * campo/terminal e diálogos lidos do DOM + o que a página passa em
   * `context`) e dispara só comandos que têm handler: sem handler, a tecla
   * segue para os componentes (composer, gaveta de aprovação, terminal).
   *
   * Embutidos: `palette.toggle` (⌘K, paleta com ações + threads + catálogo),
   * `help.toggle` (`?`, overlay com a tabela) e `nav.*` (rotas da Central).
   */
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { t } from "$lib/i18n";
  import {
    COMMANDS,
    DEFAULT_RULES,
    commandLabelKey,
    detectMac,
    domContext,
    keyLabel,
    keysFor,
    loadUserRules,
    mergeRules,
    resolve,
    type ShortcutRule,
  } from "$lib/central/shortcuts";
  import CommandPalette from "./CommandPalette.svelte";
  import ShortcutsHelp from "./ShortcutsHelp.svelte";
  import type { PaletteItem } from "./palette";

  let {
    handlers = {},
    context = () => ({}),
    threads = [],
    onopenthread,
    actions = [],
    rules: extraRules = [],
    disabled = false,
  }: {
    handlers?: Partial<Record<string, ShortcutHandler>>;
    context?: () => ShortcutsContext;
    threads?: PaletteThread[];
    onopenthread?: (id: string) => void;
    actions?: PaletteAction[];
    /** Regras extras/do usuário (substituem as padrão do mesmo comando). */
    rules?: ShortcutRule[];
    disabled?: boolean;
  } = $props();

  const isMac = detectMac();
  let paletteOpen = $state(false);
  let helpOpen = $state(false);
  let userRules = $state<ShortcutRule[]>([]);
  let rules = $derived(mergeRules(DEFAULT_RULES, [...userRules, ...extraRules]));
  let returnFocus: HTMLElement | null = null;

  const NAV: Record<string, string> = {
    "nav.catalog": "/llm/catalog",
    "nav.tools": "/llm/tools",
    "nav.sessions": "/llm/sessions",
    "nav.settings": "/settings",
  };

  function handlerFor(command: string): ShortcutHandler | null {
    const direct = handlers[command];
    if (direct) return direct;
    if (command.startsWith("thread.jump.") && handlers["thread.jump"]) {
      const n = Number(command.slice(12));
      const h = handlers["thread.jump"]!;
      return (e) => h(e, n);
    }
    if (command === "palette.toggle") return () => togglePalette();
    if (command === "help.toggle") return () => toggleHelp();
    const href = NAV[command];
    if (href) return () => void goto(href);
    return null;
  }

  function available(command: string): boolean {
    return handlerFor(command) !== null;
  }

  function remember() {
    const el = document.activeElement as HTMLElement | null;
    returnFocus = el && el !== document.body ? el : null;
  }

  function restore() {
    const el = returnFocus;
    returnFocus = null;
    if (el && document.contains(el)) queueMicrotask(() => el.focus());
  }

  export function togglePalette(open = !paletteOpen) {
    if (open === paletteOpen) return;
    if (open) {
      helpOpen = false;
      remember();
    }
    paletteOpen = open;
    if (!open) restore();
  }

  export function toggleHelp(open = !helpOpen) {
    if (open === helpOpen) return;
    if (open) {
      paletteOpen = false;
      remember();
    }
    helpOpen = open;
    if (!open) restore();
  }

  function onKeydown(e: KeyboardEvent) {
    if (disabled || e.defaultPrevented || e.isComposing) return;
    const ctx = {
      ...context(),
      ...domContext(),
      paletteOpen,
      helpOpen,
      isMac,
    };
    // Com a paleta ou a ajuda abertas, só o próprio atalho delas passa.
    const rule = resolve(rules, e, ctx, (cmd) => {
      if ((paletteOpen || helpOpen) && cmd !== "palette.toggle" && cmd !== "help.toggle") return false;
      return available(cmd);
    });
    if (!rule) return;
    const h = handlerFor(rule.command);
    if (!h) return;
    e.preventDefault();
    e.stopPropagation();
    h(e);
  }

  let paletteItems = $derived.by<PaletteItem[]>(() => {
    const out: PaletteItem[] = [];
    for (const c of COMMANDS) {
      if (c.native || c.id.startsWith("thread.jump.") || c.id === "palette.toggle") continue;
      const h = handlerFor(c.id);
      if (!h) continue;
      const k = keysFor(rules, c.id)[0];
      out.push({
        id: `cmd:${c.id}`,
        group: c.id.startsWith("nav.") ? "navigation" : "actions",
        title: $t(commandLabelKey(c.id)) as string,
        shortcut: k ? keyLabel(k.key, isMac) : undefined,
        terms: c.id,
        run: () => h(null),
      });
    }
    for (const a of actions) {
      out.push({ id: `act:${a.id}`, group: "actions", title: a.title, subtitle: a.subtitle, shortcut: a.shortcut, run: a.run });
    }
    for (const [path, key] of [
      ["/llm/threads", "llm.workspace.threads"],
      ["/llm/usage", "llm.workspace.usage"],
      ["/llm/jobs", "llm.central.shortcuts.nav.jobs"],
      ["/llm/installed", "llm.workspace.installed"],
      ["/llm/retro", "llm.workspace.retro"],
    ] as const) {
      out.push({ id: `nav:${path}`, group: "navigation", title: $t(key) as string, terms: path, secondary: true, run: () => void goto(path) });
    }
    if (onopenthread) {
      threads.forEach((th, i) => {
        out.push({
          id: `thread:${th.id}`,
          group: "threads",
          title: th.title || th.id,
          subtitle: th.subtitle,
          shortcut: i < 9 && !th.archived ? keyLabel(`mod+${i + 1}`, isMac) : undefined,
          terms: `${th.terms ?? ""} ${th.id}`,
          secondary: th.archived,
          run: () => onopenthread?.(th.id),
        });
      });
    }
    return out;
  });

  onMount(() => {
    userRules = loadUserRules();
    window.addEventListener("keydown", onKeydown, { capture: true });
    return () => window.removeEventListener("keydown", onKeydown, { capture: true });
  });
</script>

{#if paletteOpen}
  <CommandPalette items={paletteItems} onclose={() => togglePalette(false)} />
{/if}
{#if helpOpen}
  <ShortcutsHelp {rules} {isMac} {available} onclose={() => toggleHelp(false)} />
{/if}
