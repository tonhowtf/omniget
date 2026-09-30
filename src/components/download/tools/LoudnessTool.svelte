<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { open } from "@tauri-apps/plugin-dialog";
  import { t } from "$lib/i18n";

  type Result = {
    input: string;
    output: string | null;
    before_lufs: number | null;
    after_lufs: number | null;
    error: string | null;
  };

  const TARGETS = [
    { value: -14, key: "target_music" },
    { value: -16, key: "target_podcast" },
    { value: -23, key: "target_broadcast" },
  ] as const;
  const EXTENSIONS = ["mp3", "m4a", "aac", "opus", "ogg", "oga", "flac", "wav", "mp4", "m4v", "mov", "mkv", "webm"];

  let files = $state<string[]>([]);
  let target = $state<number>(-16);
  let running = $state(false);
  let results = $state<Result[]>([]);
  let error = $state<string | null>(null);

  const name = (p: string) => p.split(/[\\/]/).pop() ?? p;
  const lufs = (v: number | null) => (v == null ? "?" : `${v.toFixed(1)} LUFS`);

  async function pick() {
    const picked = await open({
      multiple: true,
      filters: [{ name: $t("downloads.tools.loudness.filter_name"), extensions: EXTENSIONS }],
    });
    if (!picked) return;
    const list = Array.isArray(picked) ? picked : [picked];
    files = [...new Set([...files, ...list])];
    results = [];
    error = null;
  }

  async function run() {
    if (running || files.length === 0) return;
    running = true;
    error = null;
    results = [];
    try {
      results = await invoke<Result[]>("normalize_loudness", { inputs: files, targetLufs: target });
    } catch (e: any) {
      const msg = typeof e === "string" ? e : "";
      error = msg === "ffmpeg unavailable" ? $t("downloads.tools.loudness.no_ffmpeg") : $t("downloads.tools.loudness.failed");
    } finally {
      running = false;
    }
  }
</script>

<div class="ln" data-agent-readiness={running ? "loading" : "ready"}>
  <p class="lead">{$t("downloads.tools.loudness.lead")}</p>

  <div class="row">
    <button class="button" onclick={pick} disabled={running}>{$t("downloads.tools.loudness.pick")}</button>
    {#if files.length > 0}
      <span class="dim">{$t("downloads.tools.loudness.count", { count: files.length })}</span>
      <button class="link" onclick={() => ((files = []), (results = []))} disabled={running}>
        {$t("downloads.tools.loudness.clear")}
      </button>
    {/if}
  </div>

  <fieldset class="targets" disabled={running}>
    <legend class="dim">{$t("downloads.tools.loudness.target")}</legend>
    {#each TARGETS as opt (opt.value)}
      <label>
        <input type="radio" name="loudness-target" value={opt.value} bind:group={target} />
        <span>{$t(`downloads.tools.loudness.${opt.key}`)}</span>
        <span class="dim small">{opt.value} LUFS</span>
      </label>
    {/each}
  </fieldset>

  <div class="row">
    <button class="button" onclick={run} disabled={running || files.length === 0}>
      {running ? $t("downloads.tools.loudness.running") : $t("downloads.tools.loudness.run")}
    </button>
    <span class="dim small">{$t("downloads.tools.loudness.keeps_original")}</span>
  </div>

  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}

  {#if files.length > 0}
    <ul class="list">
      {#each files as f (f)}
        {@const r = results.find((x) => x.input === f)}
        <li>
          <span class="name" title={f}>{name(f)}</span>
          {#if r?.error}
            <span class="error small" title={r.error}>{$t("downloads.tools.loudness.file_failed")}</span>
          {:else if r?.output}
            <span class="small">{lufs(r.before_lufs)} → {lufs(r.after_lufs)}</span>
            <button class="link small" onclick={() => void invoke("reveal_file", { path: r.output })}>{$t("downloads.tools.loudness.show")}</button>
          {:else if running}
            <span class="dim small">{$t("downloads.tools.loudness.waiting")}</span>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .ln {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .lead,
  .dim {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--text-sm);
  }
  .small {
    font-size: var(--text-xs, 11px);
  }
  .row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
  .targets {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    margin: 0;
    padding: 0;
    border: none;
  }
  .targets label {
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
    font-size: var(--text-sm);
    cursor: pointer;
  }
  .error {
    margin: 0;
    color: var(--danger, #d33);
    font-size: var(--text-sm);
  }
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }
  .list li {
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
    padding: var(--space-1) var(--space-3);
    min-width: 0;
  }
  .list li + li {
    border-top: var(--hairline) solid var(--content-border);
  }
  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--text-sm);
  }
  .link {
    border: none;
    background: none;
    padding: 0;
    color: var(--accent);
    cursor: pointer;
    font-size: var(--text-sm);
  }
</style>
