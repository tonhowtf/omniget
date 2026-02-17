<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { open } from "@tauri-apps/plugin-dialog";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import {
    getFiles,
    getOptions,
    getHwAccel,
    addFiles,
    removeFile,
    clearFiles,
    updateFileProbe,
    markFileProbing,
    markFileConverting,
    updateOptions,
    setHwAccel,
    type ProbeInfo,
    type HwAccelInfo,
  } from "$lib/stores/convert-store.svelte";

  let files = $derived(getFiles());
  let options = $derived(getOptions());
  let hwAccel = $derived(getHwAccel());
  let converting = $state(false);

  async function selectFiles() {
    const selected = await open({
      multiple: true,
      title: $t("convert.select_files"),
      filters: [
        {
          name: "Media",
          extensions: [
            "mp4", "mkv", "avi", "mov", "webm", "flv", "wmv", "m4v",
            "mp3", "wav", "flac", "aac", "ogg", "m4a", "wma", "opus",
          ],
        },
      ],
    });
    if (selected) {
      const paths = Array.isArray(selected) ? selected : [selected];
      addFiles(paths);
      for (const path of paths) {
        const file = getFiles().find((f) => f.path === path);
        if (file && file.status === "idle") {
          await probeFile(file.id, path);
        }
      }
    }
  }

  async function selectFolder() {
    const selected = await open({
      directory: true,
      title: $t("convert.select_folder"),
    });
    if (selected) {
      // Folder selection — user would need to manually add files from it
      // For now just show a toast that folder was selected
      addFiles([selected]);
    }
  }

  async function probeFile(id: number, path: string) {
    markFileProbing(id);
    try {
      const info: ProbeInfo = await invoke("probe_file", { path });
      updateFileProbe(id, info);
    } catch (e: any) {
      showToast("error", typeof e === "string" ? e : e.message ?? "Probe failed");
    }
  }

  async function loadHwAccel() {
    try {
      const info: HwAccelInfo = await invoke("get_hwaccel_info");
      setHwAccel(info);
    } catch {
      // ffmpeg not available
    }
  }

  $effect(() => {
    if (!hwAccel) {
      loadHwAccel();
    }
  });

  function getOutputPath(inputPath: string): string {
    const ext = options.outputFormat || "mp4";
    const lastDot = inputPath.lastIndexOf(".");
    const base = lastDot > 0 ? inputPath.substring(0, lastDot) : inputPath;
    return `${base}_converted.${ext}`;
  }

  async function startConversion() {
    const readyFiles = files.filter((f) => f.status === "ready" || f.status === "idle");
    if (readyFiles.length === 0) return;

    converting = true;

    for (const file of readyFiles) {
      if (!converting) break;

      const outputPath = getOutputPath(file.path);
      const convOptions = {
        input_path: file.path,
        output_path: outputPath,
        video_codec: options.videoCodec === "auto" ? null : options.videoCodec || null,
        audio_codec: options.audioCodec === "auto" ? null : options.audioCodec || null,
        resolution: options.resolution === "original" ? null : options.resolution || null,
        video_bitrate: options.videoBitrate || null,
        audio_bitrate: options.audioBitrate || null,
        sample_rate: null,
        fps: null,
        trim_start: options.trimStart || null,
        trim_end: options.trimEnd || null,
        additional_input_args: null,
        additional_output_args: null,
        preset: options.preset === "medium" ? null : options.preset || null,
      };

      try {
        const conversionId: number = await invoke("convert_file", { options: convOptions });
        markFileConverting(file.id, conversionId);

        // Wait for this conversion to complete before starting next
        await waitForConversion(conversionId);
      } catch (e: any) {
        showToast("error", typeof e === "string" ? e : e.message ?? "Conversion failed");
      }
    }

    converting = false;
  }

  function waitForConversion(conversionId: number): Promise<void> {
    return new Promise((resolve) => {
      const interval = setInterval(() => {
        const current = getFiles().find((f) => f.conversionId === conversionId);
        if (!current || current.status === "complete" || current.status === "error") {
          clearInterval(interval);
          resolve();
        }
      }, 500);
    });
  }

  async function handleCancel() {
    converting = false;
    const activeFile = files.find((f) => f.status === "converting");
    if (activeFile?.conversionId) {
      try {
        await invoke("cancel_conversion", { conversionId: activeFile.conversionId });
      } catch {}
    }
  }

  function formatDuration(seconds: number): string {
    const h = Math.floor(seconds / 3600);
    const m = Math.floor((seconds % 3600) / 60);
    const s = Math.floor(seconds % 60);
    if (h > 0) return `${h}:${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
    return `${m}:${String(s).padStart(2, "0")}`;
  }

  function formatSize(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
    return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
  }

  function getVideoStream(probe: ProbeInfo) {
    return probe.streams.find((s) => s.codec_type === "video");
  }

  function getAudioStream(probe: ProbeInfo) {
    return probe.streams.find((s) => s.codec_type === "audio");
  }
</script>

<div class="convert">
  <h2>{$t('convert.title')}</h2>

  {#if hwAccel}
    <div class="hwaccel-info">
      {#if hwAccel.recommended_video_encoder}
        <span class="hwaccel-badge detected">{$t('convert.hwaccel_detected', { encoder: hwAccel.recommended_video_encoder })}</span>
      {:else}
        <span class="hwaccel-badge none">{$t('convert.hwaccel_none')}</span>
      {/if}
    </div>
  {/if}

  <section class="section">
    <div class="actions-row">
      <button class="button" onclick={selectFiles}>{$t('convert.select_files')}</button>
      {#if files.length > 0}
        <button class="button clear-btn" onclick={clearFiles}>{$t('convert.clear_all')}</button>
      {/if}
    </div>
  </section>

  {#if files.length === 0}
    <div class="empty">
      <span class="empty-text">{$t('convert.empty')}</span>
    </div>
  {:else}
    <section class="section">
      <h5 class="section-title">{$t('convert.file_list')}</h5>
      <div class="card">
        {#each files as file, i (file.id)}
          {#if i > 0}
            <div class="divider"></div>
          {/if}
          <div class="file-row">
            <div class="file-info">
              <span class="file-name">{file.name}</span>
              {#if file.probe}
                <div class="file-meta">
                  {#if file.probe.duration_seconds > 0}
                    <span>{formatDuration(file.probe.duration_seconds)}</span>
                  {/if}
                  {#if getVideoStream(file.probe)}
                    {@const vs = getVideoStream(file.probe)}
                    {#if vs && vs.width && vs.height}
                      <span>{vs.width}x{vs.height}</span>
                    {/if}
                    {#if vs}
                      <span>{vs.codec_name}</span>
                    {/if}
                  {/if}
                  {#if getAudioStream(file.probe)}
                    {@const as_ = getAudioStream(file.probe)}
                    {#if as_}
                      <span>{as_.codec_name}</span>
                    {/if}
                  {/if}
                  <span>{formatSize(file.probe.file_size_bytes)}</span>
                </div>
              {/if}
              <div class="file-status">
                <span class="status-label" class:converting={file.status === "converting"} class:complete={file.status === "complete"} class:error={file.status === "error"}>
                  {$t(`convert.status_${file.status}`)}
                </span>
                {#if file.status === "converting"}
                  <span class="percent">{file.percent.toFixed(0)}%</span>
                {/if}
              </div>
              {#if file.status === "converting"}
                <div class="progress-bar">
                  <div class="progress-fill" style="width: {file.percent}%"></div>
                </div>
              {/if}
            </div>
            {#if file.status !== "converting"}
              <button class="button remove-btn" onclick={() => removeFile(file.id)} title={$t('convert.remove')}>
                <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <line x1="18" y1="6" x2="6" y2="18" />
                  <line x1="6" y1="6" x2="18" y2="18" />
                </svg>
              </button>
            {/if}
          </div>
        {/each}
      </div>
    </section>

    <section class="section">
      <h5 class="section-title">{$t('convert.options_title')}</h5>
      <div class="card">
        <div class="setting-row">
          <span class="setting-label">{$t('convert.output_format')}</span>
          <select class="select" value={options.outputFormat} onchange={(e) => updateOptions({ outputFormat: (e.target as HTMLSelectElement).value })}>
            <option value="mp4">MP4</option>
            <option value="mkv">MKV</option>
            <option value="webm">WebM</option>
            <option value="avi">AVI</option>
            <option value="mov">MOV</option>
            <option value="mp3">MP3</option>
            <option value="wav">WAV</option>
            <option value="flac">FLAC</option>
            <option value="aac">AAC</option>
            <option value="ogg">OGG</option>
            <option value="opus">Opus</option>
          </select>
        </div>
        <div class="divider"></div>
        <div class="setting-row">
          <span class="setting-label">{$t('convert.video_codec')}</span>
          <select class="select" value={options.videoCodec} onchange={(e) => updateOptions({ videoCodec: (e.target as HTMLSelectElement).value })}>
            <option value="auto">{$t('convert.auto')}</option>
            <option value="libx264">H.264 (x264)</option>
            <option value="libx265">H.265 (x265)</option>
            <option value="libvpx-vp9">VP9</option>
            <option value="libaom-av1">AV1</option>
            {#if hwAccel?.recommended_video_encoder}
              <option value={hwAccel.recommended_video_encoder}>{hwAccel.recommended_video_encoder} (GPU)</option>
            {/if}
            <option value="copy">Copy (no re-encode)</option>
          </select>
        </div>
        <div class="divider"></div>
        <div class="setting-row">
          <span class="setting-label">{$t('convert.audio_codec')}</span>
          <select class="select" value={options.audioCodec} onchange={(e) => updateOptions({ audioCodec: (e.target as HTMLSelectElement).value })}>
            <option value="auto">{$t('convert.auto')}</option>
            <option value="aac">AAC</option>
            <option value="libmp3lame">MP3</option>
            <option value="libvorbis">Vorbis</option>
            <option value="libopus">Opus</option>
            <option value="flac">FLAC</option>
            <option value="copy">Copy (no re-encode)</option>
          </select>
        </div>
        <div class="divider"></div>
        <div class="setting-row">
          <span class="setting-label">{$t('convert.resolution')}</span>
          <select class="select" value={options.resolution} onchange={(e) => updateOptions({ resolution: (e.target as HTMLSelectElement).value })}>
            <option value="original">{$t('convert.original')}</option>
            <option value="3840x2160">4K (3840x2160)</option>
            <option value="1920x1080">1080p</option>
            <option value="1280x720">720p</option>
            <option value="854x480">480p</option>
            <option value="640x360">360p</option>
          </select>
        </div>
        <div class="divider"></div>
        <div class="setting-row">
          <span class="setting-label">{$t('convert.preset')}</span>
          <select class="select" value={options.preset} onchange={(e) => updateOptions({ preset: (e.target as HTMLSelectElement).value })}>
            <option value="ultrafast">Ultrafast</option>
            <option value="fast">Fast</option>
            <option value="medium">Medium</option>
            <option value="slow">Slow</option>
            <option value="veryslow">Very Slow</option>
          </select>
        </div>
        <div class="divider"></div>
        <div class="setting-row">
          <span class="setting-label">{$t('convert.trim_start')}</span>
          <input
            type="text"
            class="input-text"
            placeholder="00:00:00"
            value={options.trimStart}
            onchange={(e) => updateOptions({ trimStart: (e.target as HTMLInputElement).value })}
          />
        </div>
        <div class="divider"></div>
        <div class="setting-row">
          <span class="setting-label">{$t('convert.trim_end')}</span>
          <input
            type="text"
            class="input-text"
            placeholder="00:00:00"
            value={options.trimEnd}
            onchange={(e) => updateOptions({ trimEnd: (e.target as HTMLInputElement).value })}
          />
        </div>
      </div>
    </section>

    <div class="convert-actions">
      {#if converting}
        <button class="button cancel-btn" onclick={handleCancel}>{$t('convert.cancel')}</button>
      {:else}
        <button class="button convert-btn" onclick={startConversion} disabled={files.filter((f) => f.status === "ready" || f.status === "idle").length === 0}>
          {$t('convert.convert_btn')}
        </button>
      {/if}
    </div>
  {/if}
</div>

<style>
  .convert {
    display: flex;
    flex-direction: column;
    align-items: center;
    min-height: calc(100vh - var(--padding) * 4);
    padding-top: calc(var(--padding) * 2);
    gap: calc(var(--padding) * 1.5);
  }

  .convert > :global(*) {
    width: 100%;
    max-width: 560px;
  }

  .hwaccel-info {
    display: flex;
    justify-content: center;
  }

  .hwaccel-badge {
    font-size: 12.5px;
    font-weight: 500;
    padding: calc(var(--padding) / 3) calc(var(--padding) / 1.5);
    border-radius: calc(var(--border-radius) / 2);
  }

  .hwaccel-badge.detected {
    color: var(--green);
    background: rgba(48, 189, 27, 0.1);
  }

  .hwaccel-badge.none {
    color: var(--gray);
    background: var(--button);
  }

  .section {
    display: flex;
    flex-direction: column;
    gap: calc(var(--padding) / 2);
  }

  .section-title {
    color: var(--gray);
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }

  .actions-row {
    display: flex;
    gap: calc(var(--padding) / 2);
  }

  .card {
    background: var(--button);
    box-shadow: var(--button-box-shadow);
    border-radius: var(--border-radius);
    padding: 0 calc(var(--padding) + 4px);
  }

  .divider {
    height: 1px;
    background: var(--button-stroke);
  }

  .file-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--padding);
    padding: var(--padding) 0;
  }

  .file-info {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
    flex: 1;
  }

  .file-name {
    font-size: 14.5px;
    font-weight: 500;
    color: var(--secondary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .file-meta {
    display: flex;
    gap: calc(var(--padding) / 2);
    font-size: 12.5px;
    font-weight: 500;
    color: var(--gray);
    flex-wrap: wrap;
  }

  .file-meta span:not(:last-child)::after {
    content: "\00b7";
    margin-left: calc(var(--padding) / 2);
  }

  .file-status {
    display: flex;
    align-items: center;
    gap: calc(var(--padding) / 2);
  }

  .status-label {
    font-size: 12.5px;
    font-weight: 500;
    color: var(--gray);
  }

  .status-label.converting {
    color: var(--blue);
  }

  .status-label.complete {
    color: var(--green);
  }

  .status-label.error {
    color: var(--red);
  }

  .percent {
    font-size: 12.5px;
    font-weight: 500;
    color: var(--blue);
    font-variant-numeric: tabular-nums;
  }

  .progress-bar {
    width: 100%;
    height: 6px;
    background: var(--button-elevated);
    border-radius: 3px;
    overflow: hidden;
  }

  .progress-fill {
    height: 100%;
    background: var(--blue);
    border-radius: 3px;
    transition: width 0.1s;
  }

  .remove-btn {
    padding: calc(var(--padding) / 3);
    color: var(--gray);
    flex-shrink: 0;
  }

  .remove-btn svg {
    pointer-events: none;
  }

  @media (hover: hover) {
    .remove-btn:hover {
      color: var(--red);
    }
  }

  .setting-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--padding);
    padding: calc(var(--padding) + 2px) 0;
    min-height: 48px;
  }

  .setting-label {
    font-size: 14.5px;
    font-weight: 500;
    color: var(--secondary);
  }

  .select {
    padding: calc(var(--padding) / 2) var(--padding);
    font-size: 14.5px;
    font-weight: 500;
    background: var(--button-elevated);
    border-radius: calc(var(--border-radius) / 2);
    color: var(--secondary);
    border: none;
    cursor: pointer;
    flex-shrink: 0;
  }

  .select:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  .input-text {
    width: 100px;
    padding: calc(var(--padding) / 2);
    font-size: 14.5px;
    font-weight: 500;
    text-align: center;
    background: var(--button-elevated);
    border-radius: calc(var(--border-radius) / 2);
    color: var(--secondary);
    border: 1px solid var(--input-border);
    font-variant-numeric: tabular-nums;
  }

  .input-text:focus-visible {
    border-color: var(--blue);
    outline: none;
  }

  .convert-actions {
    display: flex;
    justify-content: center;
    padding-bottom: calc(var(--padding) * 2);
  }

  .convert-btn {
    padding: calc(var(--padding) / 1.5) calc(var(--padding) * 2);
    font-size: 14.5px;
    font-weight: 500;
    background: var(--blue);
    color: #fff;
    border: none;
    border-radius: var(--border-radius);
    cursor: pointer;
  }

  .convert-btn:disabled {
    opacity: 0.5;
    cursor: default;
  }

  @media (hover: hover) {
    .convert-btn:not(:disabled):hover {
      opacity: 0.9;
    }
  }

  .convert-btn:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  .cancel-btn {
    padding: calc(var(--padding) / 1.5) calc(var(--padding) * 2);
    font-size: 14.5px;
    font-weight: 500;
    color: var(--red);
  }

  .clear-btn {
    font-size: 14.5px;
    font-weight: 500;
    color: var(--gray);
  }

  .empty {
    display: flex;
    align-items: center;
    justify-content: center;
    min-height: 200px;
  }

  .empty-text {
    font-size: 14.5px;
    font-weight: 500;
    color: var(--gray);
    text-align: center;
  }
</style>
