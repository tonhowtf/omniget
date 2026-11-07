<script lang="ts">
  import { t } from "$lib/i18n";
  import { errorAction, errorKey, formatElapsed, nearLimit, type ErrorAction } from "$lib/dictation/logic";
  import type { Mode, Platform, StateEvent } from "$lib/dictation/types";
  import Icon from "./Icon.svelte";
  import Keycaps from "./Keycaps.svelte";

  let {
    state,
    platform,
    shortcut,
    mode,
    shortcutOn,
    blocked,
    levels,
    elapsed,
    onstart,
    onstop,
    oncancel,
    onaction,
    actionBusy = false,
  }: {
    state: StateEvent;
    platform: Platform;
    shortcut: string;
    mode: Mode;
    shortcutOn: boolean;
    blocked: string | null;
    levels: number[];
    elapsed: number;
    onstart: () => void;
    onstop: () => void;
    oncancel: () => void;
    onaction: (action: ErrorAction) => void;
    actionBusy?: boolean;
  } = $props();

  let phase = $derived(state.phase);
  let err = $derived(state.error);
  let action = $derived(err ? errorAction(err.code, platform) : null);
  let warnLimit = $derived(phase === "listening" && nearLimit(elapsed, state.max_ms));
  let doneWithWarning = $derived(phase === "done" && !!err);

  let liveText = $derived.by(() => {
    if (phase === "listening") return $t("dictation.live_listening") as string;
    if (phase === "transcribing") return $t("dictation.state_transcribing") as string;
    if (phase === "done") return $t(`dictation.outcome_${state.outcome ?? "inserted"}`) as string;
    if (phase === "error" && err) return $t(`${errorKey(err.code)}_title`) as string;
    return "";
  });

  function actionLabel(a: ErrorAction): string {
    return a ? ($t(`dictation.action_${a}`) as string) : "";
  }
</script>

<section class="dc-state dc-state-{phase}" class:warn={doneWithWarning} aria-labelledby="dc-state-title">
  <p class="dc-sr" role="status" aria-live="polite">{liveText}</p>

  {#if phase === "listening"}
    <div class="dc-state-row">
      <span class="dc-orb dc-orb-live" aria-hidden="true"><Icon name="mic" size={22} /></span>
      <div class="dc-state-copy">
        <h2 id="dc-state-title">{$t("dictation.state_listening")}</h2>
        <p>
          {#if state.source === "shortcut" && mode === "hold"}
            {$t("dictation.listening_hold")}
          {:else}
            {$t("dictation.listening_toggle")}
          {/if}
        </p>
      </div>
      <div class="dc-timer" class:warn={warnLimit}>
        <span class="dc-timer-now">{formatElapsed(elapsed)}</span>
        <span class="dc-timer-max">/ {formatElapsed(state.max_ms)}</span>
      </div>
    </div>
    <div class="dc-meter" aria-hidden="true">
      {#each levels as level, i (i)}
        <span class="dc-bar" style:transform="scaleY({Math.max(0.06, level)})"></span>
      {/each}
    </div>
    {#if warnLimit}
      <p class="dc-hint">{$t("dictation.limit_soon")}</p>
    {/if}
    <div class="dc-actions">
      <button type="button" class="btn btn-primary" onclick={onstop}>
        <Icon name="stop" size={13} />
        {$t("dictation.stop_transcribe")}
      </button>
      <button type="button" class="btn btn-secondary" onclick={oncancel}>
        {$t("dictation.cancel")}
        <kbd class="kbd dc-esc">Esc</kbd>
      </button>
    </div>
  {:else if phase === "transcribing"}
    <div class="dc-state-row">
      <span class="dc-orb" aria-hidden="true"><span class="spinner dc-spin"></span></span>
      <div class="dc-state-copy">
        <h2 id="dc-state-title">{$t("dictation.state_transcribing")}</h2>
        <p>{state.notice === "MAX_REACHED" ? $t("dictation.max_reached") : $t("dictation.transcribing_body")}</p>
      </div>
    </div>
  {:else if phase === "error" && err}
    <div class="dc-state-row">
      <span class="dc-orb dc-orb-error" aria-hidden="true"><Icon name="error" size={22} /></span>
      <div class="dc-state-copy">
        <h2 id="dc-state-title">{$t(`${errorKey(err.code)}_title`)}</h2>
        <p>{$t(`${errorKey(err.code)}_body`)}</p>
        {#if err.detail}
          <details class="dc-detail">
            <summary>{$t("dictation.error_detail")}</summary>
            <code>{err.detail}</code>
          </details>
        {/if}
      </div>
    </div>
    <div class="dc-actions">
      {#if action && action !== "retry"}
        <button type="button" class="btn btn-primary" disabled={actionBusy} onclick={() => onaction(action)}>{actionLabel(action)}</button>
        <button type="button" class="btn btn-secondary" disabled={!!blocked} onclick={onstart}>{$t("dictation.try_again")}</button>
      {:else}
        <button type="button" class="btn btn-primary" disabled={!!blocked} onclick={onstart}>{$t("dictation.try_again")}</button>
      {/if}
    </div>
  {:else if phase === "done"}
    <div class="dc-state-row">
      <span class="dc-orb" class:dc-orb-done={!doneWithWarning} class:dc-orb-warn={doneWithWarning} aria-hidden="true">
        <Icon name={doneWithWarning ? "warning" : "done"} size={22} />
      </span>
      <div class="dc-state-copy">
        <h2 id="dc-state-title">{$t(`dictation.outcome_${state.outcome ?? "inserted"}`)}</h2>
        {#if doneWithWarning && err}
          <p>{$t(`${errorKey(err.code)}_body`)}</p>
        {:else}
          <p>{$t(`dictation.outcome_${state.outcome ?? "inserted"}_next`)}</p>
        {/if}
      </div>
    </div>
    {#if state.entry}
      <blockquote class="dc-quote">{state.entry.text}</blockquote>
    {/if}
    <div class="dc-actions">
      {#if doneWithWarning && action && action !== "retry"}
        <button type="button" class="btn btn-primary" onclick={() => onaction(action)}>{actionLabel(action)}</button>
        <button type="button" class="btn btn-secondary" disabled={!!blocked} onclick={onstart}>{$t("dictation.dictate_again")}</button>
      {:else}
        <button type="button" class="btn btn-primary" disabled={!!blocked} onclick={onstart}>
          <Icon name="mic" size={13} />
          {$t("dictation.dictate_again")}
        </button>
      {/if}
    </div>
  {:else}
    <div class="dc-state-row">
      <span class="dc-orb" aria-hidden="true"><Icon name="mic" size={22} /></span>
      <div class="dc-state-copy">
        <h2 id="dc-state-title">{blocked ? $t("dictation.state_blocked") : $t("dictation.state_idle")}</h2>
        {#if blocked}
          <p>{blocked}</p>
        {:else if shortcutOn}
          <p class="dc-howto">
            {mode === "hold" ? $t("dictation.idle_hold_a") : $t("dictation.idle_toggle_a")}
            <Keycaps combo={shortcut} {platform} />
            {mode === "hold" ? $t("dictation.idle_hold_b") : $t("dictation.idle_toggle_b")}
          </p>
        {:else}
          <p>{$t("dictation.idle_no_shortcut")}</p>
        {/if}
        {#if state.notice === "HOLD_HINT"}
          <p class="dc-hint">{$t("dictation.hold_hint")}</p>
        {:else if state.notice === "CANCELLED"}
          <p class="dc-hint">{$t("dictation.cancelled")}</p>
        {/if}
      </div>
    </div>
    {#if !blocked}
      <div class="dc-actions">
        <button type="button" class="btn btn-primary" onclick={onstart}>
          <Icon name="mic" size={13} />
          {$t("dictation.start_test")}
        </button>
        <span class="dc-hint">{$t("dictation.start_test_hint")}</span>
      </div>
    {/if}
  {/if}
</section>

<style>
  .dc-state {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    padding: var(--space-5);
    border-radius: var(--radius-xl);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    transition: box-shadow var(--duration-base) var(--ease-out);
  }

  .dc-state-listening {
    box-shadow: inset 0 0 0 1.5px var(--accent);
  }

  .dc-state-error {
    box-shadow: inset 0 0 0 1.5px color-mix(in srgb, var(--danger) 60%, transparent);
  }

  .dc-state.warn {
    box-shadow: inset 0 0 0 1.5px color-mix(in srgb, var(--warning) 60%, transparent);
  }

  .dc-sr {
    position: absolute;
    width: 1px;
    height: 1px;
    margin: -1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
  }

  .dc-state-row {
    display: flex;
    align-items: flex-start;
    gap: var(--space-4);
  }

  .dc-orb {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 48px;
    height: 48px;
    flex-shrink: 0;
    border-radius: var(--radius-full);
    background: var(--fill-2);
    color: var(--text-muted);
  }

  .dc-orb-live {
    background: var(--accent);
    color: var(--on-accent);
    animation: dc-pulse 1.6s var(--ease-in-out) infinite;
  }

  .dc-orb-error {
    background: color-mix(in srgb, var(--danger) 16%, transparent);
    color: var(--danger);
  }

  .dc-orb-done {
    background: color-mix(in srgb, var(--success) 16%, transparent);
    color: var(--success);
  }

  .dc-orb-warn {
    background: color-mix(in srgb, var(--warning) 18%, transparent);
    color: var(--warning);
  }

  .dc-spin {
    width: 20px;
    height: 20px;
  }

  .dc-state-copy {
    flex: 1;
    min-width: 0;
  }

  .dc-state-copy h2 {
    margin: 2px 0 var(--space-1);
    font-size: var(--text-lg);
    font-weight: 650;
    color: var(--text);
  }

  .dc-state-copy p {
    margin: 0;
    font-size: var(--text-base);
    line-height: var(--leading-base);
    color: var(--text-dim);
  }

  .dc-howto {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-1);
  }

  .dc-hint {
    margin: var(--space-2) 0 0;
    font-size: var(--text-sm);
    color: var(--text-muted);
  }

  .dc-actions .dc-hint {
    margin: 0;
  }

  .dc-timer {
    display: flex;
    align-items: baseline;
    gap: var(--space-1);
    font-variant-numeric: tabular-nums;
    color: var(--text);
  }

  .dc-timer-now {
    font-size: var(--text-2xl);
    font-weight: 600;
    letter-spacing: var(--track-tight);
  }

  .dc-timer-max {
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .dc-timer.warn .dc-timer-now {
    color: var(--warning);
  }

  .dc-meter {
    display: flex;
    align-items: center;
    gap: 3px;
    height: 40px;
    padding: 0 var(--space-1);
  }

  .dc-bar {
    flex: 1;
    height: 100%;
    border-radius: var(--radius-full);
    background: var(--accent);
    transform-origin: center;
    transition: transform 90ms linear;
  }

  .dc-actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }

  .dc-actions .btn {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
  }

  .dc-esc {
    font-size: var(--text-xs);
  }

  .dc-quote {
    margin: 0;
    padding: var(--space-3) var(--space-4);
    border-radius: var(--radius-md);
    background: var(--fill-1, var(--fill-2));
    font-size: var(--text-md);
    line-height: var(--leading-md);
    color: var(--text);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    max-height: 180px;
    overflow: auto;
  }

  .dc-detail {
    margin-top: var(--space-2);
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .dc-detail summary {
    cursor: pointer;
  }

  .dc-detail code {
    display: block;
    margin-top: var(--space-1);
    padding: var(--space-2);
    border-radius: var(--radius-sm);
    background: var(--fill-2);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  @keyframes dc-pulse {
    0%,
    100% {
      box-shadow: 0 0 0 0 color-mix(in srgb, var(--accent) 45%, transparent);
    }
    50% {
      box-shadow: 0 0 0 10px color-mix(in srgb, var(--accent) 0%, transparent);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .dc-orb-live {
      animation: none;
    }
    .dc-bar {
      transition: none;
    }
  }
</style>
