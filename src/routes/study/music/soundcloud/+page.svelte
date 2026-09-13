<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import {
    soundcloudStore,
    type ScTrack,
  } from "$lib/study-music/soundcloud-store.svelte";
  import { downloadStore } from "$lib/study-music/download-store.svelte";
  import {
    getFirstDownloadDone,
    getLastCodec,
    getLastDownloadDir,
  } from "$lib/study-music/local-prefs";
  import { colorFromString } from "$lib/study-music/format";
  import SoundCloudDownloadDialog from "$lib/study-music-components/SoundCloudDownloadDialog.svelte";
  import SoundCloudDownloadButton from "$lib/study-music-components/SoundCloudDownloadButton.svelte";
  import SoundCloudError from "$lib/study-music-components/SoundCloudError.svelte";

  let booting = $state(true);
  let downloadTrack = $state<ScTrack | null>(null);

  let importOpen = $state(false);
  let importUrl = $state("");
  let importBusy = $state(false);
  let importError = $state<string | null>(null);

  let loginBusy = $state(false);
  let loginError = $state<string | null>(null);

  let loadError = $state<string | null>(null);
  let playbackError = $state<string | null>(null);
  let playbackTrack = $state<ScTrack | null>(null);
  let playbackQueue = $state<ScTrack[] | null>(null);
  let recommended = $state<ScTrack[]>([]);
  let recommendedSeed = $state<ScTrack | null>(null);

  onMount(async () => {
    await boot();
  });

  async function boot() {
    loadError = null;
    try {
      await soundcloudStore.refreshStatus();
      if (soundcloudStore.isLoggedIn) {
        await soundcloudStore.loadAll();
        void loadRecommended();
      }
    } catch (e) {
      loadError = e instanceof Error ? e.message : String(e);
    } finally {
      booting = false;
    }
  }

  async function loadRecommended() {
    const seed = soundcloudStore.likedTracks[0] ?? soundcloudStore.streamFeed[0];
    if (!seed) return;
    try {
      const tracks = await soundcloudStore.relatedTracks(seed.id, 12);
      recommended = tracks;
      recommendedSeed = seed;
    } catch (e) {
      console.warn("[soundcloud] related failed", e);
    }
  }

  async function refresh() {
    loadError = null;
    try {
      await soundcloudStore.refreshStatus();
      await soundcloudStore.loadAll();
      void loadRecommended();
    } catch (e) {
      loadError = e instanceof Error ? e.message : String(e);
    }
  }

  async function handleLogin() {
    if (loginBusy) return;
    loginBusy = true;
    loginError = null;
    try {
      await soundcloudStore.loginWithWebview();
      if (soundcloudStore.isLoggedIn) {
        void loadRecommended();
      } else {
        loginError = $t("study.music.sc.login_failed");
      }
    } catch (e) {
      loginError = e instanceof Error ? e.message : String(e);
    } finally {
      loginBusy = false;
    }
  }

  async function handleLogout() {
    try {
      await soundcloudStore.logout();
      recommended = [];
      recommendedSeed = null;
      loginError = null;
    } catch (e) {
      showToast("error", e instanceof Error ? e.message : String(e));
    }
  }

  async function play(track: ScTrack, queue?: ScTrack[]) {
    playbackError = null;
    playbackTrack = track;
    playbackQueue = queue ?? [track];
    try {
      await soundcloudStore.playTrack(track, queue);
    } catch (e) {
      playbackError = e instanceof Error ? e.message : String(e);
    }
  }

  function retryPlayback() {
    if (!playbackTrack) return;
    void play(playbackTrack, playbackQueue ?? [playbackTrack]);
  }

  function fmtDuration(ms: number): string {
    const total = Math.floor(ms / 1000);
    const m = Math.floor(total / 60);
    const s = total % 60;
    return `${m}:${s.toString().padStart(2, "0")}`;
  }

  function folderName(dir: string): string {
    const parts = dir.split(/[\\/]/).filter(Boolean);
    return parts[parts.length - 1] ?? dir;
  }

  async function startInlineDownload(track: ScTrack) {
    const codec = getLastCodec() ?? "mp3";
    const dir = getLastDownloadDir();
    if (!dir) {
      downloadTrack = track;
      return;
    }
    const optId = downloadStore.addOptimisticSingleJob({
      trackId: track.id,
      title: track.title,
      artist: track.user.username,
      artwork:
        soundcloudStore.pickArtwork(track.artwork_url ?? track.user.avatar_url) ?? null,
    });
    try {
      await soundcloudStore.download({
        trackId: track.id,
        codec,
        outputDir: dir,
        quality: "progressive",
      });
      showToast("success", `Pronto — salvo na pasta ${folderName(dir)}`);
    } catch (e) {
      downloadStore.markJobError(optId, e instanceof Error ? e.message : String(e));
    }
  }

  function handleDownload(track: ScTrack, advanced: boolean) {
    if (advanced || !getFirstDownloadDone() || !getLastDownloadDir()) {
      downloadTrack = track;
      return;
    }
    void startInlineDownload(track);
  }

  function openImport() {
    importUrl = "";
    importError = null;
    importOpen = true;
  }

  function closeImport() {
    importOpen = false;
  }

  async function submitImport() {
    if (importBusy) return;
    const url = importUrl.trim();
    if (!url || !url.includes("soundcloud.com")) {
      importError = $t("study.music.sc_import_invalid") as string;
      return;
    }
    importBusy = true;
    importError = null;
    try {
      const res = await soundcloudStore.resolveAnyUrl(url);
      if (res.kind === "track" && res.data?.id) {
        const track: any = res.data;
        const queue: ScTrack[] = [
          {
            id: track.id,
            title: track.title ?? "",
            duration: track.duration ?? 0,
            artwork_url: track.artwork_url ?? null,
            permalink_url: track.permalink_url ?? "",
            streamable: track.streamable ?? true,
            downloadable: track.downloadable ?? false,
            user: {
              id: track.user?.id ?? 0,
              username: track.user?.username ?? "",
              avatar_url: track.user?.avatar_url ?? null,
              permalink: track.user?.permalink ?? "",
              permalink_url: track.user?.permalink_url ?? "",
            },
            playback_count: track.playback_count ?? 0,
            likes_count: track.likes_count ?? 0,
          },
        ];
        await play(queue[0], queue);
        showToast(
          "success",
          $t("study.music.sc_import_success", { title: res.title || track.title }) as string,
        );
        importOpen = false;
      } else if (res.kind === "playlist" && res.data?.id) {
        importOpen = false;
        showToast(
          "success",
          $t("study.music.sc_import_success", { title: res.title }) as string,
        );
        goto(`/study/music/soundcloud/playlist/${res.data.id}`);
      } else if ((res.kind === "user" || res.kind === "user-resource") && res.data?.id) {
        importOpen = false;
        goto(`/study/music/soundcloud/user/${res.data.id}`);
      } else {
        importError = $t("study.music.sc_import_invalid") as string;
      }
    } catch (e) {
      const err = e instanceof Error ? e.message : String(e);
      importError = $t("study.music.sc_import_failed", { error: err }) as string;
    } finally {
      importBusy = false;
    }
  }
</script>

<section class="sc-home">
  <header class="page-head">
    <h1>{$t('study.music.sc_title')}</h1>
    {#if soundcloudStore.isLoggedIn && soundcloudStore.profile}
      <div class="head-actions">
        <button type="button" class="cta-import" onclick={openImport}>
          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <line x1="12" y1="5" x2="12" y2="19"/>
            <line x1="5" y1="12" x2="19" y2="12"/>
          </svg>
          {$t('study.music.sc_import_url')}
        </button>
        <button type="button" class="ghost-btn" onclick={() => goto("/study/music/soundcloud/search")}>
          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <circle cx="11" cy="11" r="8"/>
            <line x1="21" y1="21" x2="16.65" y2="16.65"/>
          </svg>
          {$t('study.music.sc_search')}
        </button>
        <button type="button" class="ghost-btn" onclick={refresh} title={$t('study.music.sc_refresh') as string} aria-label={$t('study.music.sc_refresh') as string}>
          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <polyline points="23 4 23 10 17 10"/>
            <path d="M20.49 15a9 9 0 1 1-2.12-9.36L23 10"/>
          </svg>
        </button>
        <span class="profile-pill">
          {#if soundcloudStore.profile.avatar_url}
            <img src={soundcloudStore.pickArtwork(soundcloudStore.profile.avatar_url)} alt="" />
          {/if}
          <span>{soundcloudStore.profile.username}</span>
        </span>
        <button type="button" class="ghost-btn" onclick={handleLogout}>{$t('study.music.sc_logout')}</button>
      </div>
    {/if}
  </header>

  {#if booting}
    <p class="muted">{$t('study.music.sc_loading')}</p>
  {:else if !soundcloudStore.isLoggedIn}
    <div class="login-card">
      <div class="logo">
        <svg viewBox="0 0 24 24" width="80" height="80" fill="#ff5500" aria-hidden="true">
          <path d="M1.175 12.225c-.051 0-.094.046-.101.1l-.233 2.154.233 2.105c.007.058.05.098.101.098.05 0 .09-.04.099-.098l.255-2.105-.27-2.154c0-.057-.045-.1-.09-.1m-.899.828c-.06 0-.091.037-.104.094L0 14.479l.165 1.308c0 .055.045.094.09.094s.089-.045.104-.104l.21-1.319-.21-1.334c0-.061-.044-.1-.09-.1m1.83-1.229c-.061 0-.12.045-.12.104l-.21 2.563.225 2.458c0 .06.045.12.12.12.06 0 .12-.061.12-.12l.255-2.458-.27-2.563c0-.06-.045-.104-.12-.104m.945-.089c-.075 0-.135.06-.15.135l-.193 2.652.21 2.544c.016.077.075.135.149.135.075 0 .135-.058.15-.135l.225-2.544-.225-2.652c-.014-.075-.075-.135-.149-.135m1.139.07c-.089 0-.164.072-.164.163l-.21 2.602.193 2.581c0 .088.075.157.164.157.09 0 .166-.069.18-.157l.21-2.581-.21-2.602c-.014-.091-.09-.163-.18-.163m.93-.058c-.105 0-.18.07-.18.166l-.21 2.6.18 2.539c0 .106.075.179.179.179.111 0 .19-.073.19-.18l.224-2.539-.21-2.6c0-.094-.083-.165-.197-.165m.95-.082c-.12 0-.21.087-.21.207l-.165 2.6.165 2.482c0 .12.09.214.21.214.12 0 .211-.094.211-.214l.196-2.482-.196-2.6c0-.12-.091-.207-.21-.207m1.022-.034c-.135 0-.226.103-.226.225l-.165 2.616.165 2.466c0 .118.09.222.226.222.12 0 .21-.104.226-.222l.179-2.466-.18-2.616c-.015-.122-.105-.225-.225-.225m1.039-.225c-.135 0-.255.121-.255.255l-.135 2.84.135 2.453c0 .135.12.241.255.241s.255-.106.27-.241l.149-2.453-.15-2.84c-.014-.135-.135-.255-.27-.255m1.171.106c-.149 0-.27.121-.27.271l-.119 2.738.119 2.453c0 .15.121.27.27.27.15 0 .271-.12.286-.27l.135-2.453-.135-2.738c-.015-.15-.135-.27-.286-.271m1.205.345c-.165 0-.299.135-.299.301l-.105 2.379.105 2.469c0 .166.134.301.299.301.165 0 .3-.135.3-.3l.121-2.471-.121-2.379c0-.166-.135-.301-.3-.301m1.295-.106c-.18 0-.314.135-.314.314l-.091 2.47.091 2.453c0 .181.135.315.314.315.181 0 .315-.134.315-.315l.105-2.453-.105-2.47c0-.179-.135-.314-.315-.314m1.376-.241c-.196 0-.345.149-.345.345l-.075 2.692.075 2.452c0 .197.15.345.345.345.196 0 .346-.148.346-.345l.09-2.452-.09-2.692c0-.196-.151-.345-.346-.345m1.487 5.879H22.49a1.515 1.515 0 0 0 1.51-1.524 1.512 1.512 0 0 0-1.51-1.51c-.135 0-.27.014-.39.044C22.04 9.706 19.94 7.766 17.39 7.766c-.61 0-1.21.135-1.756.36-.15.06-.18.105-.18.255v9.117c.014.165.135.299.299.314z"/>
        </svg>
      </div>
      <h2>Entrar com SoundCloud</h2>
      <p>Vai abrir uma janelinha do SoundCloud. Faça login e ela fecha sozinha quando você terminar.</p>
      <div class="login-actions">
        <button type="button" class="cta" disabled={loginBusy} onclick={handleLogin}>
          {loginBusy ? "Abrindo SoundCloud…" : "Entrar com SoundCloud"}
        </button>
      </div>
      {#if loginError}
        <p class="error">{loginError}</p>
        <details class="why-fail">
          <summary>Por que pode falhar?</summary>
          <ul>
            <li>Conta nova: confirme o email primeiro no SoundCloud</li>
            <li>2FA: termine o segundo passo antes de fechar a janela</li>
            <li>Captcha: resolva no site, a janela espera você</li>
          </ul>
        </details>
      {/if}
    </div>
  {:else}
    {#if loadError}
      <SoundCloudError error={loadError} onRetry={refresh} />
    {/if}
    {#if playbackError}
      <SoundCloudError
        error={playbackError}
        trackUrl={playbackTrack?.permalink_url}
        onRetry={retryPlayback}
      />
    {/if}
    {#if recommended.length > 0}
      <section class="block">
        <header class="block-head">
          <h2>{$t('study.music.sc_section_recommended', { name: recommendedSeed?.title ?? $t('study.music.sc_recommended_seed_fallback') })}</h2>
        </header>
        <div class="track-list">
          {#each recommended as track (track.id)}
            <div class="track-row" role="button" tabindex="0" onclick={() => play(track, recommended)} onkeydown={(e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); play(track, recommended); } }}>
              <div class="track-cover">
                {#if track.artwork_url || track.user.avatar_url}
                  <img src={soundcloudStore.pickArtwork(track.artwork_url ?? track.user.avatar_url)} alt="" loading="lazy" />
                {:else}
                  <div class="track-fallback" style:background={colorFromString(track.title)}></div>
                {/if}
              </div>
              <div class="track-meta">
                <span class="track-title">{track.title}</span>
                <span class="track-artist">{track.user.username}</span>
              </div>
              <span class="track-dur">{fmtDuration(track.duration)}</span>
              <SoundCloudDownloadButton {track} onTrigger={handleDownload} />
            </div>
          {/each}
        </div>
      </section>
    {/if}

    {#if soundcloudStore.streamFeed.length > 0}
      <section class="block">
        <header class="block-head"><h2>{$t('study.music.sc_section_stream')}</h2></header>
        <div class="track-list">
          {#each soundcloudStore.streamFeed.slice(0, 12) as track (track.id)}
            <div class="track-row" role="button" tabindex="0" onclick={() => play(track, soundcloudStore.streamFeed)} onkeydown={(e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); play(track, soundcloudStore.streamFeed); } }}>
              <div class="track-cover">
                {#if track.artwork_url || track.user.avatar_url}
                  <img src={soundcloudStore.pickArtwork(track.artwork_url ?? track.user.avatar_url)} alt="" loading="lazy" />
                {:else}
                  <div class="track-fallback" style:background={colorFromString(track.title)}></div>
                {/if}
              </div>
              <div class="track-meta">
                <span class="track-title">{track.title}</span>
                <span class="track-artist">{track.user.username}</span>
              </div>
              <span class="track-dur">{fmtDuration(track.duration)}</span>
              <SoundCloudDownloadButton {track} onTrigger={handleDownload} />
            </div>
          {/each}
        </div>
      </section>
    {/if}

    {#if soundcloudStore.likedTracks.length > 0}
      <section class="block">
        <header class="block-head"><h2>{$t('study.music.sc_section_likes', { count: soundcloudStore.likedTracks.length })}</h2></header>
        <div class="track-list">
          {#each soundcloudStore.likedTracks.slice(0, 30) as track, i (track.id)}
            <div class="track-row" role="button" tabindex="0" onclick={() => play(track, soundcloudStore.likedTracks)} onkeydown={(e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); play(track, soundcloudStore.likedTracks); } }}>
              <span class="track-num">{i + 1}</span>
              <div class="track-cover">
                {#if track.artwork_url || track.user.avatar_url}
                  <img src={soundcloudStore.pickArtwork(track.artwork_url ?? track.user.avatar_url)} alt="" loading="lazy" />
                {/if}
              </div>
              <div class="track-meta">
                <span class="track-title">{track.title}</span>
                <span class="track-artist">{track.user.username}</span>
              </div>
              <span class="track-dur">{fmtDuration(track.duration)}</span>
              <SoundCloudDownloadButton {track} onTrigger={handleDownload} />
            </div>
          {/each}
        </div>
      </section>
    {/if}

    {#if soundcloudStore.playlists.length > 0}
      <section class="block">
        <header class="block-head"><h2>{$t('study.music.sc_section_playlists')}</h2></header>
        <div class="grid">
          {#each soundcloudStore.playlists as p (p.id)}
            <a class="card" href={`/study/music/soundcloud/playlist/${p.id}`}>
              <div class="card-cover">
                {#if p.artwork_url}
                  <img src={soundcloudStore.pickArtwork(p.artwork_url)} alt="" loading="lazy" />
                {:else}
                  <div class="card-fallback" style:background={colorFromString(p.title)}></div>
                {/if}
              </div>
              <h3 class="card-title">{p.title}</h3>
              <p class="card-sub">{$t('study.music.sc_track_count', { count: p.track_count })} · {p.user.username}</p>
            </a>
          {/each}
        </div>
      </section>
    {/if}

    {#if soundcloudStore.followings.length > 0}
      <section class="block">
        <header class="block-head"><h2>{$t('study.music.sc_section_following')}</h2></header>
        <div class="grid">
          {#each soundcloudStore.followings as u (u.id)}
            <a class="card" href={`/study/music/soundcloud/user/${u.id}`}>
              <div class="card-circle">
                {#if u.avatar_url}
                  <img src={soundcloudStore.pickArtwork(u.avatar_url)} alt="" loading="lazy" />
                {:else}
                  <div class="card-fallback round" style:background={colorFromString(u.username)}>{u.username.slice(0,1).toUpperCase()}</div>
                {/if}
              </div>
              <h3 class="card-title center">{u.username}</h3>
              {#if u.followers_count}<p class="card-sub center">{$t('study.music.sc_followers_count', { count: u.followers_count.toLocaleString() })}</p>{/if}
            </a>
          {/each}
        </div>
      </section>
    {/if}
  {/if}
</section>

{#if downloadTrack}
  <SoundCloudDownloadDialog track={downloadTrack} onClose={() => (downloadTrack = null)} />
{/if}

{#if importOpen}
  <div class="overlay" role="presentation" onclick={(e) => { if (e.target === e.currentTarget) closeImport(); }}>
    <div class="dialog" role="dialog" aria-modal="true">
      <header class="d-head">
        <h3>{$t('study.music.sc_import_url_title')}</h3>
        <button type="button" class="d-close" onclick={closeImport} aria-label="close">×</button>
      </header>
      <div class="d-body">
        <p class="d-hint">{$t('study.music.sc_import_url_desc')}</p>
        <input
          type="url"
          class="d-input"
          placeholder={$t('study.music.sc_import_url_placeholder') as string}
          bind:value={importUrl}
          onkeydown={(e) => { if (e.key === 'Enter') submitImport(); }}
          spellcheck="false"
        />
        {#if importError}<p class="d-error">{importError}</p>{/if}
      </div>
      <footer class="d-foot">
        <button type="button" class="ghost-btn" onclick={closeImport}>{$t('common.cancel')}</button>
        <button type="button" class="cta" disabled={importBusy || !importUrl.trim()} onclick={submitImport}>
          {importBusy ? $t('study.music.sc_loading') : $t('study.music.sc_import_action')}
        </button>
      </footer>
    </div>
  </div>
{/if}

<style>
  .sc-home { display: flex; flex-direction: column; gap: 32px; color: rgba(255,255,255,0.95); }
  .page-head { display: flex; align-items: center; justify-content: space-between; gap: 16px; flex-wrap: wrap; }
  .page-head h1 { margin: 0; font-size: clamp(28px,3.5vw,40px); font-weight: 900; letter-spacing: -0.02em; }
  .head-actions { display: flex; align-items: center; gap: 12px; }
  .profile-pill { display: inline-flex; align-items: center; gap: 8px; padding: 4px 12px 4px 4px; background: rgba(255,255,255,0.05); border-radius: 999px; font-size: 13px; font-weight: 600; }
  .profile-pill img { width: 28px; height: 28px; border-radius: 50%; object-fit: cover; }
  .ghost-btn { display: inline-flex; align-items: center; gap: 6px; padding: 7px 16px; background: rgba(255,255,255,0.05); border: 0; border-radius: 999px; color: rgba(255,255,255,0.85); font-family: inherit; font-size: 12px; font-weight: 600; cursor: pointer; transition: background 200ms ease; }
  .ghost-btn:hover { background: rgba(255,255,255,0.1); }
  .cta-import { display: inline-flex; align-items: center; gap: 6px; padding: 8px 16px; background: #ff5500; color: white; border: 0; border-radius: 999px; font-family: inherit; font-size: 12px; font-weight: 700; cursor: pointer; transition: background 200ms ease; }
  .cta-import:hover { background: #ff7733; }
  .overlay { position: fixed; inset: 0; background: rgba(0,0,0,0.65); display: grid; place-items: center; z-index: 999; }
  .dialog { background: #1c1c1c; border: none; border-radius: 12px; width: min(480px, 92vw); display: flex; flex-direction: column; }
  .d-head { display: flex; justify-content: space-between; align-items: center; padding: 16px 20px; border-bottom: none; }
  .d-head h3 { margin: 0; font-size: 16px; font-weight: 700; }
  .d-close { background: transparent; border: 0; color: rgba(255,255,255,0.6); font-size: 22px; line-height: 1; cursor: pointer; padding: 0 8px; }
  .d-body { padding: 16px 20px; display: flex; flex-direction: column; gap: 12px; }
  .d-hint { margin: 0; color: rgba(255,255,255,0.6); font-size: 13px; line-height: 1.5; }
  .d-input { width: 100%; padding: 12px 14px; background: rgba(255,255,255,0.05); border: none; border-radius: 8px; color: white; font-family: inherit; font-size: 14px; box-sizing: border-box; }
  .d-input:focus { outline: 0; border-color: #ff5500; }
  .d-error { margin: 0; color: #e22134; font-size: 13px; }
  .d-foot { display: flex; justify-content: flex-end; gap: 8px; padding: 12px 20px; border-top: none; }
  .d-foot .cta { padding: 9px 22px; }
  .d-foot .cta:disabled { opacity: 0.5; cursor: not-allowed; }

  .login-card { margin: 0 auto; max-width: 540px; padding: 56px 32px; text-align: center; display: flex; flex-direction: column; align-items: center; gap: 16px; }
  .login-card h2 { margin: 0; font-size: 28px; font-weight: 800; }
  .login-card p { margin: 0; color: rgba(255,255,255,0.6); font-size: 14px; line-height: 1.6; max-width: 44ch; }
  .login-actions { display: flex; gap: 12px; flex-wrap: wrap; justify-content: center; margin-top: 8px; }
  .cta { padding: 12px 32px; background: #ff5500; color: white; border: 0; border-radius: 999px; font-family: inherit; font-size: 14px; font-weight: 700; cursor: pointer; transition: background 200ms ease; }
  .cta:hover { background: #ff7733; }
  .error { color: #e22134; font-size: 13px; }
  .why-fail { margin-top: 4px; max-width: 44ch; text-align: left; color: rgba(255,255,255,0.65); font-size: 12px; }
  .why-fail summary { cursor: pointer; color: rgba(255,255,255,0.55); padding: 2px 0; user-select: none; }
  .why-fail summary:hover { color: rgba(255,255,255,0.85); }
  .why-fail ul { margin: 8px 0 0; padding-left: 18px; display: flex; flex-direction: column; gap: 4px; }
  .why-fail li { line-height: 1.5; }
  .cta:disabled { opacity: 0.6; cursor: not-allowed; }

  .block { display: flex; flex-direction: column; gap: 16px; }
  .block-head h2 { margin: 0; font-size: 22px; font-weight: 800; }
  .track-list { display: flex; flex-direction: column; gap: 2px; }
  .track-row { display: grid; grid-template-columns: 28px 56px 1fr 60px 32px; gap: 12px; align-items: center; padding: 8px 12px; background: transparent; border: 0; border-radius: 6px; color: inherit; cursor: pointer; text-align: left; transition: background 200ms ease; }
  .track-row:hover { background: rgba(255,255,255,0.06); }
  .track-num { color: rgba(255,255,255,0.5); font-size: 12px; text-align: center; }
  .track-cover { width: 44px; height: 44px; border-radius: 4px; overflow: hidden; background: rgba(40,40,40,0.8); }
  .track-cover img { width: 100%; height: 100%; object-fit: cover; }
  .track-fallback { width: 100%; height: 100%; }
  .track-meta { display: flex; flex-direction: column; min-width: 0; }
  .track-title { font-size: 14px; font-weight: 600; color: white; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .track-artist { font-size: 12px; color: rgba(255,255,255,0.55); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .track-dur { font-size: 13px; color: rgba(255,255,255,0.55); text-align: right; font-variant-numeric: tabular-nums; }

  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(160px, 1fr)); gap: 24px; }
  .card { background: transparent; border: 0; padding: 0; cursor: pointer; color: inherit; text-align: left; text-decoration: none; }
  .card-cover, .card-circle { aspect-ratio: 1/1; border-radius: 12px; overflow: hidden; margin-bottom: 12px; background: rgba(40,40,40,0.8); }
  .card-circle { border-radius: 50%; }
  .card-cover img, .card-circle img { width: 100%; height: 100%; object-fit: cover; transition: transform 500ms ease; }
  .card:hover .card-cover img, .card:hover .card-circle img { transform: scale(1.05); }
  .card-fallback { width: 100%; height: 100%; display: grid; place-items: center; color: white; font-size: 32px; font-weight: 800; }
  .card-fallback.round { border-radius: 50%; }
  .card-title { margin: 0; font-size: 14px; font-weight: 700; color: white; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; padding: 0 4px; }
  .card-title.center { text-align: center; }
  .card-sub { margin: 4px 0 0; font-size: 12px; color: rgba(255,255,255,0.5); padding: 0 4px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .card-sub.center { text-align: center; }

  .muted { color: rgba(255,255,255,0.5); font-size: 13px; }
  @media (prefers-reduced-motion: reduce) { .card-cover img, .card-circle img { transition: none; } .card:hover .card-cover img, .card:hover .card-circle img { transform: none; } }
</style>
