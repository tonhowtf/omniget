<script lang="ts">
  // Thread composer: `/` commands, `$` skills, `@` files, `#` PRs (menu grows
  // out of the top), context chips (files, skills, PRs, terminal selections,
  // diff comments, attachments), driver/instance + model, access mode and
  // plan mode, queue while a turn runs (⌘Enter sends now), ↑/↓ prompt
  // history, per-thread draft. A drawer slot on top holds approvals,
  // questions and the plan follow-up.
  import { tick, untrack, type Snippet } from "svelte";
  import { t } from "$lib/i18n";
  import type { AccessMode, InstanceView, ThreadRow } from "$lib/central/threads/types";
  import { threadsUi, newId, type Chip } from "$lib/central/threads-ui/ui-state.svelte";
  import { BUILTIN_COMMANDS, detectTrigger, rank, replaceTrigger, recallable, type Trigger } from "$lib/central/threads-ui/composer";
  import { prLookup, searchFiles } from "$lib/central/threads-ui/api";
  import { ACCESS_MODES, MODEL_HINTS, accessIcon, driverName, driverTint } from "$lib/central/threads-ui/drivers";
  import { basename, dirname } from "$lib/central/threads-ui/format";
  import { getSkills, loadSkills } from "$lib/stores/llm-skills-store.svelte";
  import type { IconName } from "$lib/central/threads-ui/icons";
  import Icon from "./Icon.svelte";
  import SnapShotButton from "$components/central/preview/SnapShotButton.svelte";

  type MenuItem = { id: string; icon: IconName; label: string; desc?: string; badge?: string; apply: () => void | Promise<void> };

  let {
    thread,
    instances,
    cwd,
    running,
    blocked = false,
    history = [],
    text = $bindable(""),
    drawer,
    meter,
    onsend,
    onsteer,
    onstop,
    oncommand,
  }: {
    thread: ThreadRow;
    instances: InstanceView[];
    cwd: string | null;
    running: boolean;
    /** Approval / question pending: sending waits. */
    blocked?: boolean;
    history?: string[];
    text?: string;
    drawer?: Snippet;
    meter?: Snippet;
    onsend: (text: string, chips: Chip[]) => Promise<boolean>;
    onsteer: (text: string, chips: Chip[]) => Promise<boolean>;
    onstop: () => void;
    oncommand: (name: string) => void;
  } = $props();

  const MAX_HEIGHT = 240;
  let chips = $state<Chip[]>([]);
  let area = $state<HTMLTextAreaElement | null>(null);
  let trigger = $state<Trigger | null>(null);
  let items = $state<MenuItem[]>([]);
  let active = $state(0);
  let searching = $state(false);
  let sending = $state(false);
  let historyPos = $state<number | null>(null);
  let accessOpen = $state(false);
  let modelDraft = $state("");
  let searchSeq = 0;
  let searchTimer: ReturnType<typeof setTimeout> | null = null;
  const modelListId = $props.id();

  // Load the thread's draft on switch; save as the user types.
  let loadedFor = "";
  $effect(() => {
    const id = thread.threadId;
    untrack(() => {
      if (loadedFor === id) return;
      loadedFor = id;
      const d = threadsUi.draft(id);
      text = d.text;
      chips = [...d.chips];
      historyPos = null;
      trigger = null;
      modelDraft = thread.model ?? "";
      void tick().then(autosize);
    });
  });

  // Keep the model field in sync with the thread unless the user is editing it.
  $effect(() => {
    const m = thread.model ?? "";
    untrack(() => {
      if (document.activeElement?.id !== `${modelListId}-input`) modelDraft = m;
    });
  });

  function persist() {
    threadsUi.setDraft(thread.threadId, { text, chips: $state.snapshot(chips) as Chip[] });
  }

  function autosize() {
    const el = area;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, MAX_HEIGHT)}px`;
  }

  // ── Context insertion (public) ────────────────────────────────────────

  /** Adds a context chip (terminal selection, diff comment, file…). */
  export function addChip(chip: Omit<Chip, "id"> & { id?: string }) {
    chips = [...chips, { ...chip, id: chip.id ?? newId(chip.kind) }];
    persist();
    focus();
  }

  /** Puts text into the draft (appends; used by "edit from here", plan refine). */
  export function insertText(s: string, replace = false) {
    text = replace || !text.trim() ? s : `${text.replace(/\s+$/, "")}\n\n${s}`;
    persist();
    void tick().then(() => {
      autosize();
      focus(true);
    });
  }

  export function focus(toEnd = false) {
    area?.focus();
    if (toEnd && area) area.selectionStart = area.selectionEnd = area.value.length;
  }

  export function getDraft(): { text: string; chips: Chip[] } {
    return { text, chips: $state.snapshot(chips) as Chip[] };
  }

  export function clear() {
    text = "";
    chips = [];
    persist();
    void tick().then(autosize);
  }

  function removeChip(id: string) {
    chips = chips.filter((c) => c.id !== id);
    persist();
  }

  // ── Triggers ──────────────────────────────────────────────────────────

  function refreshTrigger() {
    const el = area;
    if (!el) return;
    const next = detectTrigger(text, el.selectionStart ?? text.length);
    const changed = next?.kind !== trigger?.kind || next?.query !== trigger?.query;
    trigger = next;
    if (changed) void buildItems();
  }

  function apply(insert: string) {
    if (!trigger) return;
    const r = replaceTrigger(text, trigger, insert);
    text = r.text;
    trigger = null;
    items = [];
    persist();
    void tick().then(() => {
      if (area) {
        area.focus();
        area.selectionStart = area.selectionEnd = r.caret;
      }
      autosize();
    });
  }

  function removeTriggerToken() {
    if (!trigger) return;
    text = text.slice(0, trigger.start) + text.slice(trigger.end);
    trigger = null;
    items = [];
    persist();
  }

  async function buildItems() {
    const trig = trigger;
    active = 0;
    if (!trig) {
      items = [];
      return;
    }
    const seq = ++searchSeq;
    searching = false;
    if (trig.kind === "command") {
      const empty = text.trim() === `/${trig.query}` && !chips.length;
      const builtins = rank(
        BUILTIN_COMMANDS.filter((c) => c.only !== "empty" || empty),
        trig.query,
        (c) => c.name,
        (c) => $t(c.descKey) as string,
      ).map<MenuItem>((c) => ({
        id: `cmd:${c.name}`,
        icon: "lightning",
        label: `/${c.name}`,
        desc: $t(c.descKey) as string,
        apply: () => {
          removeTriggerToken();
          oncommand(c.name);
        },
      }));
      await loadSkills().catch(() => undefined);
      if (seq !== searchSeq) return;
      const skills = rank(getSkills(), trig.query.replace(/^skill:/, ""), (s) => s.name, (s) => s.description).slice(0, 20).map<MenuItem>((s) => ({
        id: `skill:${s.name}`,
        icon: "sparkle",
        label: `/skill:${s.name}`,
        desc: s.description,
        badge: $t("llm.central.threads.composer.skill") as string,
        apply: () => pickSkill(s.name, s.description),
      }));
      items = [...builtins, ...skills];
      return;
    }
    if (trig.kind === "skill") {
      await loadSkills().catch(() => undefined);
      if (seq !== searchSeq) return;
      items = rank(getSkills(), trig.query, (s) => s.name, (s) => s.description).slice(0, 30).map((s) => ({
        id: `skill:${s.name}`,
        icon: "sparkle" as IconName,
        label: `$${s.name}`,
        desc: s.description,
        apply: () => pickSkill(s.name, s.description),
      }));
      return;
    }
    if (trig.kind === "file") {
      if (!cwd || !trig.query) {
        items = [];
        return;
      }
      searching = true;
      if (searchTimer) clearTimeout(searchTimer);
      searchTimer = setTimeout(async () => {
        const hits = await searchFiles(cwd!, trig.query);
        if (seq !== searchSeq) return;
        searching = false;
        items = hits.map((h) => ({
          id: `file:${h.path}`,
          icon: (h.isDir ? "folder-simple" : "file-text") as IconName,
          label: basename(h.path),
          desc: dirname(h.path),
          apply: () => pickFile(h.path),
        }));
      }, 120);
      return;
    }
    if (trig.kind === "pr") {
      if (!cwd) {
        items = [];
        return;
      }
      searching = true;
      if (searchTimer) clearTimeout(searchTimer);
      searchTimer = setTimeout(async () => {
        const ref = /^\d+$/.test(trig.query) ? trig.query : undefined;
        const pr = await prLookup(cwd!, ref);
        if (seq !== searchSeq) return;
        searching = false;
        const ok = pr && (!trig.query || ref || pr.title.toLowerCase().includes(trig.query.toLowerCase()) || pr.head.includes(trig.query));
        items = ok && pr
          ? [{
              id: `pr:${pr.number}`,
              icon: "git-pull-request",
              label: `#${pr.number} ${pr.title}`,
              desc: `${pr.state}${pr.draft ? " · draft" : ""} · ${pr.head} → ${pr.base}`,
              apply: () => {
                apply(`#${pr.number}`);
                addChip({
                  kind: "pr",
                  label: `#${pr.number}`,
                  title: pr.title,
                  value: `Pull request #${pr.number}: ${pr.title}\nState: ${pr.state}${pr.draft ? " (draft)" : ""}\nBranch: ${pr.head} → ${pr.base}\nURL: ${pr.url}`,
                });
              },
            }]
          : [];
      }, 180);
    }
  }

  function pickSkill(name: string, description: string) {
    apply(`$${name}`);
    chips = [...chips, { id: newId("skill"), kind: "skill", label: name, title: description, value: `$${name}` }];
    persist();
  }

  function pickFile(path: string) {
    apply(`@${path}`);
  }

  // ── Keyboard ──────────────────────────────────────────────────────────

  async function submit(steer = false) {
    const body = text.trim();
    if ((!body && !chips.length) || sending) return;
    if (/^\/(plan|default)\s*$/i.test(body) && !chips.length) {
      oncommand(body.slice(1).trim().toLowerCase());
      clear();
      return;
    }
    sending = true;
    const snapshotText = text;
    const snapshotChips = $state.snapshot(chips) as Chip[];
    try {
      const ok = steer ? await onsteer(snapshotText, snapshotChips) : await onsend(snapshotText, snapshotChips);
      if (ok && text === snapshotText) clear();
      historyPos = null;
    } finally {
      sending = false;
    }
  }

  function caretOnFirstLine(): boolean {
    const el = area;
    if (!el) return false;
    return !text.slice(0, el.selectionStart ?? 0).includes("\n");
  }

  function caretOnLastLine(): boolean {
    const el = area;
    if (!el) return false;
    return !text.slice(el.selectionEnd ?? text.length).includes("\n");
  }

  function recall(dir: -1 | 1): boolean {
    if (!history.length || chips.length) return false;
    if (historyPos == null) {
      if (dir === 1 || text.trim()) return false;
      historyPos = history.length - 1;
    } else {
      const next = historyPos + dir;
      if (next >= history.length) {
        historyPos = null;
        text = "";
        persist();
        return true;
      }
      historyPos = Math.max(0, next);
    }
    text = recallable(history[historyPos]);
    persist();
    void tick().then(() => {
      autosize();
      if (area) area.selectionStart = area.selectionEnd = dir === -1 ? 0 : text.length;
    });
    return true;
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.isComposing) return;
    const menuOpen = !!trigger && (items.length > 0 || searching);
    if (menuOpen) {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        active = (active + 1) % Math.max(1, items.length);
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        active = (active - 1 + items.length) % Math.max(1, items.length);
        return;
      }
      if ((e.key === "Enter" || e.key === "Tab") && items[active]) {
        e.preventDefault();
        void items[active].apply();
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        trigger = null;
        items = [];
        return;
      }
    }
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      void submit(running && (e.metaKey || e.ctrlKey));
      return;
    }
    if (e.key === "." && (e.metaKey || e.ctrlKey) && running) {
      e.preventDefault();
      onstop();
      return;
    }
    if (e.key === "ArrowUp" && !e.shiftKey && !e.altKey && !e.metaKey && caretOnFirstLine() && !blocked) {
      if (recall(-1)) e.preventDefault();
      return;
    }
    if (e.key === "ArrowDown" && !e.shiftKey && !e.altKey && !e.metaKey && historyPos != null && caretOnLastLine()) {
      if (recall(1)) e.preventDefault();
    }
  }

  function onInput() {
    if (historyPos != null) historyPos = null;
    autosize();
    persist();
    refreshTrigger();
  }

  // ── Attachments ───────────────────────────────────────────────────────

  async function attach() {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const picked = await open({ multiple: true, directory: false, defaultPath: cwd ?? undefined });
      const list = Array.isArray(picked) ? picked : picked ? [picked] : [];
      for (const p of list) {
        const path = String(p);
        addChip({ kind: "attachment", label: basename(path), path, value: path });
      }
    } catch {
      /* dialog unavailable */
    }
  }

  function onDrop(e: DragEvent) {
    const files = e.dataTransfer?.files;
    if (!files?.length) return;
    e.preventDefault();
    for (const f of Array.from(files)) {
      const path = (f as File & { path?: string }).path;
      if (path) addChip({ kind: "attachment", label: f.name, path, value: path });
    }
  }

  // ── Pickers ───────────────────────────────────────────────────────────

  let inst = $derived(instances.find((i) => i.instance.id === thread.instanceId) ?? null);
  let hints = $derived(MODEL_HINTS[thread.driver] ?? []);

  export function focusModel() {
    document.getElementById(`${modelListId}-input`)?.focus();
  }

  function commitModel() {
    const m = modelDraft.trim();
    if (m === (thread.model ?? "")) return;
    oncommand(`model:${m}`);
  }

  function chipIcon(k: Chip["kind"]): IconName {
    switch (k) {
      case "file":
        return "file-text";
      case "skill":
        return "sparkle";
      case "pr":
        return "git-pull-request";
      case "terminal":
        return "terminal-window";
      case "review":
        return "git-diff";
      case "attachment":
        return "paperclip";
      default:
        return "hash";
    }
  }

  let placeholder = $derived(
    blocked
      ? ($t("llm.central.threads.composer.placeholder_blocked") as string)
      : running
        ? ($t("llm.central.threads.composer.placeholder_running") as string)
        : thread.interactionMode === "plan"
          ? ($t("llm.central.threads.composer.placeholder_plan") as string)
          : ($t("llm.central.threads.composer.placeholder") as string),
  );
  let hasContent = $derived(text.trim().length > 0 || chips.length > 0);
</script>

<div class="composer-wrap">
  {#if drawer}{@render drawer()}{/if}

  {#if trigger && (items.length || searching)}
    <div class="menu" role="listbox" aria-label={$t("llm.central.threads.composer.suggestions")}>
      {#if searching && !items.length}
        <p class="menu-empty">{$t(`llm.central.threads.composer.searching_${trigger.kind}`)}</p>
      {/if}
      {#each items as it, i (it.id)}
        <button
          type="button"
          role="option"
          aria-selected={i === active}
          class="menu-item"
          class:active={i === active}
          onmousedown={(e) => e.preventDefault()}
          onmousemove={() => (active = i)}
          onclick={() => it.apply()}
        >
          <Icon name={it.icon} size={14} />
          <span class="mi-label">{it.label}</span>
          {#if it.badge}<span class="mi-badge">{it.badge}</span>{/if}
          {#if it.desc}<span class="mi-desc">{it.desc}</span>{/if}
        </button>
      {/each}
    </div>
  {/if}

  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="composer" class:plan={thread.interactionMode === "plan"} ondragover={(e) => e.preventDefault()} ondrop={onDrop}>
    {#if chips.length}
      <div class="chips">
        {#each chips as c (c.id)}
          <span class="chip" data-kind={c.kind} title={c.title ?? c.value.slice(0, 400)}>
            <Icon name={chipIcon(c.kind)} size={11} />
            <span class="chip-label">{c.label}</span>
            <button type="button" class="chip-x" aria-label={$t("llm.central.threads.composer.remove_chip", { label: c.label })} onclick={() => removeChip(c.id)}><Icon name="x" size={9} /></button>
          </span>
        {/each}
      </div>
    {/if}
    <textarea
      bind:this={area}
      bind:value={text}
      rows="1"
      class="input-area"
      {placeholder}
      aria-label={$t("llm.central.threads.composer.label")}
      oninput={onInput}
      onkeydown={onKeydown}
      onclick={refreshTrigger}
      onkeyup={(e) => {
        if (e.key === "ArrowLeft" || e.key === "ArrowRight") refreshTrigger();
      }}
    ></textarea>

    <div class="bar">
      <button type="button" class="tool" aria-label={$t("llm.central.threads.composer.attach")} title={$t("llm.central.threads.composer.attach")} onmousedown={(e) => e.preventDefault()} onclick={attach}>
        <Icon name="paperclip" size={15} />
      </button>
      <SnapShotButton threadId={thread.threadId} onattach={(path) => addChip({ kind: "attachment", label: basename(path), path, value: path })} />

      <span class="inst" style:--tint={thread.driver ? driverTint(thread.driver, inst) : "var(--text-muted)"}>
        <span class="dot"></span>
        <select
          aria-label={$t("llm.central.threads.composer.instance")}
          value={thread.instanceId}
          disabled={running}
          onchange={(e) => oncommand(`instance:${e.currentTarget.value}`)}
        >
          {#each instances as i (i.instance.id)}
            <option value={i.instance.id} disabled={!i.available}>{driverName(i.instance.driver, i)}{i.instance.accountId ? ` · ${i.instance.accountId}` : ""}</option>
          {/each}
          {#if !inst}<option value={thread.instanceId}>{thread.instanceId}</option>{/if}
        </select>
      </span>

      <input
        id="{modelListId}-input"
        class="model"
        list={modelListId}
        bind:value={modelDraft}
        placeholder={$t("llm.central.threads.composer.model_default")}
        aria-label={$t("llm.central.threads.composer.model")}
        disabled={running}
        onchange={commitModel}
        onkeydown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            commitModel();
            focus();
          }
        }}
      />
      <datalist id={modelListId}>{#each hints as h (h)}<option value={h}></option>{/each}</datalist>

      <span class="access-wrap">
        <button type="button" class="tool labeled" aria-haspopup="menu" aria-expanded={accessOpen} title={$t(`llm.central.threads.access_hint.${thread.runtimeMode}`)} onclick={() => (accessOpen = !accessOpen)}>
          <Icon name={accessIcon(thread.runtimeMode)} size={14} /><span class="lbl">{$t(`llm.central.threads.access.${thread.runtimeMode}`)}</span>
        </button>
        {#if accessOpen}
          <div class="access-menu" role="menu">
            {#each ACCESS_MODES as m (m)}
              <button type="button" role="menuitemradio" aria-checked={m === thread.runtimeMode} onclick={() => {
                accessOpen = false;
                oncommand(`access:${m as AccessMode}`);
              }}>
                <Icon name={accessIcon(m)} size={14} />
                <span><strong>{$t(`llm.central.threads.access.${m}`)}</strong><small>{$t(`llm.central.threads.access_hint.${m}`)}</small></span>
                {#if m === thread.runtimeMode}<Icon name="check" size={13} />{/if}
              </button>
            {/each}
          </div>
        {/if}
      </span>

      <button
        type="button"
        class="tool labeled"
        class:on={thread.interactionMode === "plan"}
        aria-pressed={thread.interactionMode === "plan"}
        title={$t("llm.central.threads.composer.plan_hint")}
        onclick={() => oncommand(thread.interactionMode === "plan" ? "default" : "plan")}
      >
        <Icon name="list-checks" size={14} /><span class="lbl">{$t("llm.central.threads.composer.plan")}</span>
      </button>

      <span class="grow"></span>
      {#if meter}{@render meter()}{/if}

      {#if running}
        <button type="button" class="stop" aria-label={$t("llm.central.threads.composer.stop")} title={`${$t("llm.central.threads.composer.stop")} (⌘.)`} onclick={onstop}>
          <span class="sq"></span>
        </button>
        {#if hasContent}
          <button type="button" class="send" disabled={sending} title={$t("llm.central.threads.composer.queue_hint")} onclick={() => submit(false)}>
            <Icon name="queue" size={15} /><span class="lbl">{$t("llm.central.threads.composer.queue")}</span>
          </button>
        {/if}
      {:else}
        <button type="button" class="send" disabled={!hasContent || sending} aria-label={$t("llm.central.threads.composer.send")} onclick={() => submit(false)}>
          <Icon name="arrow-up" size={16} />
        </button>
      {/if}
    </div>
  </div>
</div>

<style>
  .composer-wrap { position: relative; max-width: 800px; width: 100%; margin: 0 auto; padding: 0 20px 14px 36px; box-sizing: border-box; }
  .composer { border: 1px solid var(--separator); border-radius: 18px; background: var(--fill-1); box-shadow: 0 2px 10px color-mix(in srgb, var(--text) 5%, transparent); padding: 8px 10px 8px; display: grid; gap: 6px; }
  .composer:focus-within { border-color: color-mix(in srgb, var(--accent) 55%, var(--separator)); }
  .composer.plan { border-color: color-mix(in srgb, var(--purple, #8b5cf6) 45%, var(--separator)); }
  .composer-wrap:has(> :global(section)) .composer { border-top-left-radius: 0; border-top-right-radius: 0; }
  .input-area { width: 100%; resize: none; border: 0; outline: none; background: transparent; color: var(--text); font: inherit; font-size: 14.5px; line-height: 1.5; min-height: 44px; max-height: 240px; padding: 6px 6px 0; box-sizing: border-box; }
  .chips { display: flex; flex-wrap: wrap; gap: 4px; padding: 2px 4px 0; }
  .chip { display: inline-flex; align-items: center; gap: 4px; max-width: 260px; font-size: 11.5px; padding: 2px 4px 2px 7px; border-radius: 7px; background: var(--accent-soft); color: var(--accent-hi); }
  .chip[data-kind="terminal"] { background: color-mix(in srgb, var(--teal, #14b8a6) 14%, transparent); color: var(--teal, #0f766e); }
  .chip[data-kind="review"] { background: color-mix(in srgb, var(--orange, #f59e0b) 14%, transparent); color: var(--orange, #b45309); }
  .chip-label { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-family: var(--font-mono); }
  .chip-x { border: 0; background: transparent; color: inherit; padding: 2px; cursor: pointer; display: inline-flex; border-radius: 4px; opacity: 0.7; }
  .chip-x:hover { opacity: 1; background: color-mix(in srgb, currentColor 15%, transparent); }
  .bar { display: flex; align-items: center; gap: 4px; min-height: 32px; flex-wrap: wrap; }
  .tool { display: inline-flex; align-items: center; gap: 5px; height: 28px; min-width: 28px; justify-content: center; border: 0; background: transparent; color: var(--text-muted); border-radius: 8px; cursor: pointer; font-size: 12px; padding: 0 6px; }
  .tool:hover { background: var(--fill-2); color: var(--text); }
  .tool.on { color: var(--purple, #8b5cf6); background: color-mix(in srgb, var(--purple, #8b5cf6) 12%, transparent); }
  @container (max-width: 560px) { .labeled .lbl { display: none; } }
  .inst { display: inline-flex; align-items: center; gap: 5px; padding-left: 6px; border-radius: 8px; }
  .inst:hover { background: var(--fill-2); }
  .dot { width: 8px; height: 8px; border-radius: 50%; background: var(--tint); }
  .inst select { border: 0; background: transparent; color: var(--text); font-size: 12px; height: 28px; max-width: 150px; cursor: pointer; padding-right: 2px; }
  .model { width: 120px; border: 0; background: transparent; color: var(--text-muted); font-size: 12px; font-family: var(--font-mono); height: 28px; border-radius: 8px; padding: 0 6px; }
  .model:hover, .model:focus { background: var(--fill-2); color: var(--text); outline: none; }
  .access-wrap { position: relative; }
  .access-menu { position: absolute; bottom: calc(100% + 6px); left: 0; z-index: 30; width: 290px; padding: 4px; border-radius: 12px; background: var(--popup-bg, var(--surface)); border: 1px solid var(--separator); box-shadow: 0 12px 32px color-mix(in srgb, #000 22%, transparent); display: grid; }
  .access-menu button { display: flex; align-items: center; gap: 10px; border: 0; background: transparent; color: var(--text); padding: 7px 8px; border-radius: 8px; cursor: pointer; text-align: left; }
  .access-menu button:hover, .access-menu button:focus-visible { background: var(--accent-soft); }
  .access-menu span { flex: 1; display: grid; gap: 1px; }
  .access-menu strong { font-size: 12.5px; font-weight: 600; }
  .access-menu small { font-size: 11px; color: var(--text-muted); }
  .grow { flex: 1; }
  .send { display: inline-flex; align-items: center; justify-content: center; gap: 5px; min-width: 32px; height: 32px; border-radius: 16px; border: 0; background: var(--accent); color: var(--on-accent, #fff); cursor: pointer; padding: 0 9px; font-size: 12px; font-weight: 600; }
  .send:disabled { opacity: 0.35; cursor: default; }
  .stop { width: 32px; height: 32px; border-radius: 50%; border: 0; background: var(--red, #ef4444); cursor: pointer; display: inline-flex; align-items: center; justify-content: center; transition: transform 120ms; }
  .stop:hover { transform: scale(1.06); }
  .sq { width: 11px; height: 11px; border-radius: 3px; background: #fff; }
  .menu { position: absolute; left: 36px; right: 20px; bottom: calc(100% - 10px); z-index: 25; max-height: 18rem; overflow: auto; padding: 4px; border-radius: 14px; background: var(--popup-bg, var(--surface)); border: 1px solid var(--separator); box-shadow: 0 -8px 30px color-mix(in srgb, #000 16%, transparent); }
  .menu-item { display: flex; align-items: center; gap: 8px; width: 100%; border: 0; background: transparent; color: var(--text); padding: 6px 8px; border-radius: 8px; cursor: pointer; text-align: left; font-size: 13px; }
  .menu-item.active { background: var(--accent-soft); }
  .mi-label { font-weight: 500; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; max-width: 55%; }
  .mi-badge { font-size: 10px; padding: 0 5px; border-radius: 4px; background: var(--fill-2); color: var(--text-muted); }
  .mi-desc { color: var(--text-muted); font-size: 12px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; flex: 1; }
  .menu-empty { margin: 6px 8px; color: var(--text-muted); font-size: 12.5px; }
  .composer-wrap { container-type: inline-size; }
</style>
