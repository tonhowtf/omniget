<script lang="ts">
  import SettingsField from "./SettingsField.svelte";
  import { pluginInvoke } from "$lib/plugin-invoke";

  type PotokenVisitor = {
    has_token: boolean;
    expires_in_seconds: number | null;
    expired: boolean | null;
  };

  type PotokenStatus = {
    engine: string;
    snapshot_version: string;
    bgutils_version: string;
    minting_available: boolean;
    minting_status: string;
    visitor: PotokenVisitor;
    content_cached_count: number;
    last_generated_at: number | null;
  };

  type ClientStatus = {
    ready: boolean;
    decipher_engine: string;
    potoken_supported: boolean;
    potoken_minting: string;
    bgutils_version: string;
    js_engine: string;
    player_cache_id: string | null;
    player_cache_age_seconds: number | null;
  };

  type TestVideoResult = {
    client: string;
    success: boolean;
    status: string;
    reason: string | null;
    has_direct_url: boolean;
    has_cipher: boolean;
    format_count: number;
    adaptive_count: number;
    audio_format_count: number;
    best_audio_itag: number | null;
    best_audio_bitrate: number | null;
    expires_in_seconds: number | null;
    use_signature_timestamp: boolean;
    use_web_po_tokens: boolean;
    error: string | null;
  };

  type TestVideoResponse = {
    video_id: string;
    any_success: boolean;
    any_direct_url: boolean;
    any_audio_format: boolean;
    first_working_client: string | null;
    cascade_length: number;
    results: TestVideoResult[];
  };

  let clientStatus = $state<ClientStatus | null>(null);
  let potokenStatus = $state<PotokenStatus | null>(null);
  let loading = $state(false);
  let lastError = $state<string | null>(null);
  let lastInfo = $state<string | null>(null);

  let testVideoId = $state("");
  let testRunning = $state(false);
  let testResult = $state<TestVideoResponse | null>(null);

  let manualVisitorToken = $state("");
  let manualContentToken = $state("");
  let manualContentVideoId = $state("");
  let manualPanelOpen = $state(false);

  async function refresh() {
    loading = true;
    lastError = null;
    try {
      const status = await pluginInvoke<ClientStatus>("study", "youtube:client_status", {});
      clientStatus = status;
      const pot = await pluginInvoke<PotokenStatus>("study", "youtube:potoken_status", {});
      potokenStatus = pot;
    } catch (e) {
      lastError = String(e);
    } finally {
      loading = false;
    }
  }

  async function invalidatePlayerCache() {
    loading = true;
    lastError = null;
    lastInfo = null;
    try {
      const r = await pluginInvoke<{ player_js_rows_deleted: number }>(
        "study",
        "youtube:player_invalidate_cache",
        {},
      );
      lastInfo = `Cache do player.js limpo (${r.player_js_rows_deleted} linha(s)).`;
      await refresh();
    } catch (e) {
      lastError = String(e);
    } finally {
      loading = false;
    }
  }

  async function clearPotokens() {
    loading = true;
    lastError = null;
    lastInfo = null;
    try {
      const r = await pluginInvoke<{ deleted: number }>("study", "youtube:potoken_clear", {});
      lastInfo = `PoTokens removidos: ${r.deleted}.`;
      await refresh();
    } catch (e) {
      lastError = String(e);
    } finally {
      loading = false;
    }
  }

  async function saveManualToken() {
    loading = true;
    lastError = null;
    lastInfo = null;
    try {
      const r = await pluginInvoke<{ visitor_saved: boolean; content_saved: boolean }>(
        "study",
        "youtube:potoken_set_manual",
        {
          visitor_token: manualVisitorToken.trim() || undefined,
          content_token: manualContentToken.trim() || undefined,
          video_id: manualContentVideoId.trim() || undefined,
        },
      );
      lastInfo = `Tokens salvos. visitor=${r.visitor_saved} content=${r.content_saved}.`;
      manualVisitorToken = "";
      manualContentToken = "";
      manualContentVideoId = "";
      manualPanelOpen = false;
      await refresh();
    } catch (e) {
      lastError = String(e);
    } finally {
      loading = false;
    }
  }

  async function runTestVideo() {
    if (!testVideoId.trim()) {
      lastError = "Cole um video ID ou URL de YouTube.";
      return;
    }
    testRunning = true;
    lastError = null;
    testResult = null;
    try {
      testResult = await pluginInvoke<TestVideoResponse>("study", "youtube:test_video", {
        video_id: testVideoId.trim(),
      });
    } catch (e) {
      lastError = String(e);
    } finally {
      testRunning = false;
    }
  }

  function formatTokenStatus(v: PotokenVisitor | undefined): string {
    if (!v || !v.has_token) return "ausente";
    if (v.expired) return "expirado";
    const s = v.expires_in_seconds ?? 0;
    const h = Math.floor(s / 3600);
    const m = Math.floor((s % 3600) / 60);
    return `válido por ${h}h${m}min`;
  }

  $effect(() => {
    void refresh();
  });
</script>

<section class="tab">
  <SettingsField label="Cliente YouTube" description="Estado do plugin study para reprodução de áudio do YouTube/Music.">
    {#if loading && !clientStatus}
      <span class="muted">Carregando…</span>
    {:else if clientStatus}
      <dl class="status-grid">
        <dt>Decipher engine</dt><dd><code>{clientStatus.decipher_engine}</code></dd>
        <dt>JS engine</dt><dd><code>{clientStatus.js_engine}</code></dd>
        <dt>bgutils-js</dt><dd><code>{clientStatus.bgutils_version}</code></dd>
        <dt>PoToken minting</dt><dd><code>{clientStatus.potoken_minting}</code></dd>
        <dt>Player.js cache</dt>
        <dd>
          {#if clientStatus.player_cache_id}
            <code>{clientStatus.player_cache_id}</code>
            {#if clientStatus.player_cache_age_seconds !== null}
              <span class="muted"> · {Math.floor(clientStatus.player_cache_age_seconds / 60)}min atrás</span>
            {/if}
          {:else}
            <span class="muted">vazio</span>
          {/if}
        </dd>
      </dl>
    {/if}
    <div class="row">
      <button class="btn" disabled={loading} onclick={invalidatePlayerCache}>Limpar cache do player.js</button>
    </div>
  </SettingsField>

  <SettingsField label="PoToken" description="Token de autenticação assinado pela YouTube. Gerado automaticamente via bgutils-js no boa_engine.">
    {#if potokenStatus}
      <dl class="status-grid">
        <dt>Mint disponível</dt><dd><code>{potokenStatus.minting_available ? "sim" : "não"}</code></dd>
        <dt>Visitor token</dt><dd>{formatTokenStatus(potokenStatus.visitor)}</dd>
        <dt>Content tokens em cache</dt><dd><code>{potokenStatus.content_cached_count}</code></dd>
      </dl>
    {/if}
    <div class="row">
      <button class="btn" disabled={loading} onclick={clearPotokens}>Limpar PoTokens</button>
      <button class="btn ghost" onclick={() => (manualPanelOpen = !manualPanelOpen)}>
        {manualPanelOpen ? "Fechar token manual" : "Cole token manual…"}
      </button>
    </div>
    {#if manualPanelOpen}
      <div class="manual-panel">
        <p class="hint">
          Capture do DevTools (F12) em <code>music.youtube.com</code>: aba Network, request POST <code>youtubei/v1/player</code>, body tem
          <code>serviceIntegrityDimensions.poToken</code> (content) e <code>context.user.visitorData</code>.
        </p>
        <label class="field">
          <span>Visitor token (opcional)</span>
          <input type="text" placeholder="CgsxxxxxYAk%3D…" bind:value={manualVisitorToken} />
        </label>
        <label class="field">
          <span>Content token (opcional, requer video ID)</span>
          <input type="text" placeholder="MnQxxxxxxxxx…" bind:value={manualContentToken} />
        </label>
        <label class="field">
          <span>Video ID (para content token)</span>
          <input type="text" placeholder="dQw4w9WgXcQ" bind:value={manualContentVideoId} />
        </label>
        <div class="row">
          <button class="btn primary" disabled={loading} onclick={saveManualToken}>Salvar</button>
        </div>
      </div>
    {/if}
  </SettingsField>

  <SettingsField label="Testar vídeo" description="Tenta cada cliente do cascade e mostra qual conseguiu cifra/stream. Útil pra debug.">
    <div class="test-row">
      <input
        type="text"
        placeholder="Cole video ID (dQw4w9WgXcQ) ou URL completa"
        bind:value={testVideoId}
        disabled={testRunning}
      />
      <button class="btn primary" disabled={testRunning || !testVideoId.trim()} onclick={runTestVideo}>
        {testRunning ? "Testando…" : "Testar"}
      </button>
    </div>
    {#if testResult}
      <div class="test-summary">
        <strong>{testResult.video_id}</strong> ·
        {#if testResult.first_working_client}
          primeiro cliente OK: <code>{testResult.first_working_client}</code>
        {:else}
          <span class="error">nenhum cliente funcionou</span>
        {/if}
      </div>
      <table class="cascade-table">
        <thead>
          <tr>
            <th>Cliente</th>
            <th>Status</th>
            <th>Audio</th>
            <th>Direto</th>
            <th>Cifra</th>
            <th>sts/pot</th>
            <th>Detalhe</th>
          </tr>
        </thead>
        <tbody>
          {#each testResult.results as r}
            <tr class:ok={r.success} class:err={!r.success && !!r.error}>
              <td><code>{r.client}</code></td>
              <td>{r.status}</td>
              <td>
                {#if r.audio_format_count > 0}
                  {r.audio_format_count}
                  {#if r.best_audio_itag}
                    <span class="muted">itag {r.best_audio_itag}</span>
                  {/if}
                {:else}
                  <span class="muted">—</span>
                {/if}
              </td>
              <td>{r.has_direct_url ? "✓" : "—"}</td>
              <td>{r.has_cipher ? "✓" : "—"}</td>
              <td class="muted">{r.use_signature_timestamp ? "sts" : ""} {r.use_web_po_tokens ? "pot" : ""}</td>
              <td class="muted small">{r.reason || r.error || ""}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </SettingsField>

  {#if lastError}
    <p class="error-message">{lastError}</p>
  {/if}
  {#if lastInfo}
    <p class="info-message">{lastInfo}</p>
  {/if}
</section>

<style>
  .tab {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .row {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
    margin-top: 8px;
  }
  .btn {
    padding: 6px 12px;
    border-radius: 6px;
    border: 1px solid color-mix(in oklab, var(--text) 18%, transparent);
    background: var(--bg);
    color: var(--text);
    cursor: pointer;
    font-size: var(--text-sm);
  }
  .btn:hover:not(:disabled) {
    background: color-mix(in oklab, var(--text) 6%, transparent);
  }
  .btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .btn.primary {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--on-accent);
  }
  .btn.ghost {
    background: transparent;
  }
  .status-grid {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 4px 16px;
    font-size: var(--text-sm);
    margin: 0;
  }
  .status-grid dt { color: var(--secondary); }
  .status-grid dd { margin: 0; }
  code {
    font-family: ui-monospace, monospace;
    font-size: 11.5px;
    padding: 1px 5px;
    background: color-mix(in oklab, var(--text) 8%, transparent);
    border-radius: 3px;
  }
  .muted { color: var(--secondary); }
  .small { font-size: 11px; }
  .hint {
    font-size: 11.5px;
    color: var(--secondary);
    line-height: 1.5;
    margin: 8px 0;
  }
  .manual-panel {
    margin-top: 10px;
    padding: 12px;
    background: color-mix(in oklab, var(--text) 4%, transparent);
    border-radius: 6px;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin-bottom: 8px;
    font-size: 12px;
  }
  .field input {
    padding: 6px 10px;
    border-radius: 5px;
    border: 1px solid color-mix(in oklab, var(--text) 14%, transparent);
    background: var(--bg);
    color: var(--text);
    font-family: ui-monospace, monospace;
    font-size: 11.5px;
  }
  .field span { color: var(--secondary); }
  .test-row {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .test-row input {
    flex: 1;
    padding: 6px 10px;
    border-radius: 5px;
    border: 1px solid color-mix(in oklab, var(--text) 14%, transparent);
    background: var(--bg);
    color: var(--text);
    font-size: var(--text-sm);
  }
  .test-summary {
    font-size: 13px;
    margin: 10px 0;
  }
  .test-summary .error { color: var(--danger); }
  .cascade-table {
    width: 100%;
    border-collapse: collapse;
    font-size: 11.5px;
    margin-top: 8px;
  }
  .cascade-table th,
  .cascade-table td {
    text-align: left;
    padding: 4px 6px;
    border-bottom: 1px solid color-mix(in oklab, var(--text) 8%, transparent);
  }
  .cascade-table th {
    color: var(--secondary);
    font-weight: 500;
  }
  .cascade-table tr.ok {
    background: color-mix(in oklab, var(--accent) 5%, transparent);
  }
  .cascade-table tr.err {
    color: var(--secondary);
  }
  .error-message {
    color: var(--danger);
    font-size: 12px;
    margin: 0;
  }
  .info-message {
    color: var(--accent);
    font-size: 12px;
    margin: 0;
  }
</style>
