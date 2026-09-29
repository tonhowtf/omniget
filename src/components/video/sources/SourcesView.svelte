<script lang="ts">
  // Sources: connection cards (Connect Integration Cards on 21st.dev) for
  // Jellyfin/Emby, IPTV (M3U or Xtream), add-ons and AniList, then the list of
  // what is connected with status, on/off and remove (confirmed in the page).
  // Nothing ships pre-installed: every source is one the user brings.
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import * as api from "$lib/video/api";
  import { goRoot } from "$lib/video/ui.svelte";
  import type { AddonInfo, AniViewer, SourceInfo, SourceKind } from "$lib/video/types";
  import Modal from "$components/music/Modal.svelte";
  import Head from "../views/parts/Head.svelte";
  import Skel from "../views/parts/Skel.svelte";
  import VIcon from "../views/parts/VIcon.svelte";
  import { errText } from "../views/parts/util";

  /** AniList client for the implicit token flow. Empty = send the user to their developer settings. */
  const ANILIST_CLIENT_ID = "";
  const ANILIST_TOKEN_URL = ANILIST_CLIENT_ID
    ? `https://anilist.co/api/v2/oauth/authorize?client_id=${ANILIST_CLIENT_ID}&response_type=token`
    : "https://anilist.co/settings/developer";

  type Form = "jellyfin" | "m3u" | "xtream" | "addon" | "anilist";
  type Card = { id: Form; icon: string; title: string; body: string; sends: string };

  let sources = $state<SourceInfo[] | null>(null);
  let addonList = $state<AddonInfo[]>([]);
  let viewer = $state<AniViewer | null>(null);
  let error = $state<string | null>(null);
  let form = $state<Form | null>(null);
  let busy = $state(false);
  let formError = $state<string | null>(null);
  let confirmId = $state<string | null>(null);
  let refreshing = $state<number | null>(null);

  // form fields
  let jfServer = $state("");
  let jfUser = $state("");
  let jfPass = $state("");
  let qc = $state<{ code: string; secret: string } | null>(null);
  let qcTimer: ReturnType<typeof setInterval> | null = null;
  let m3uName = $state("");
  let m3uMode = $state<"url" | "file">("url");
  let m3uUrl = $state("");
  let m3uFile = $state("");
  let m3uEpg = $state("");
  let xtName = $state("");
  let xtServer = $state("");
  let xtUser = $state("");
  let xtPass = $state("");
  let addonUrl = $state("");
  let aniToken = $state("");

  let cards = $derived<Card[]>([
    { id: "jellyfin", icon: "server", title: $t("video.sources.jf_title") as string, body: $t("video.sources.jf_body") as string, sends: $t("video.sources.jf_sends") as string },
    { id: "m3u", icon: "live", title: $t("video.sources.m3u_title") as string, body: $t("video.sources.m3u_body") as string, sends: $t("video.sources.m3u_sends") as string },
    { id: "xtream", icon: "live", title: $t("video.sources.xt_title") as string, body: $t("video.sources.xt_body") as string, sends: $t("video.sources.xt_sends") as string },
    { id: "addon", icon: "addons", title: $t("video.sources.addon_title") as string, body: $t("video.sources.addon_body") as string, sends: $t("video.sources.addon_sends") as string },
    { id: "anilist", icon: "anime", title: $t("video.sources.ani_title") as string, body: $t("video.sources.ani_body") as string, sends: $t("video.sources.ani_sends") as string },
  ]);

  async function load() {
    try {
      const [s, a, v] = await Promise.all([api.sources(), api.addons().catch(() => [] as AddonInfo[]), api.anilistViewer().catch(() => null)]);
      sources = s;
      addonList = a;
      viewer = v;
      error = null;
    } catch (e) {
      error = errText(e);
      sources ??= [];
    }
  }

  onMount(() => void load());
  onDestroy(stopQc);

  // add-ons that the backend doesn't list as sources still need a row
  let looseAddons = $derived(sources && !sources.some((s) => s.kind === "addon") ? addonList : []);

  function openForm(f: Form) {
    form = f;
    formError = null;
    busy = false;
  }
  function closeForm() {
    stopQc();
    form = null;
    formError = null;
  }

  async function run(fn: () => Promise<unknown>, okMsg: string) {
    busy = true;
    formError = null;
    try {
      await fn();
      showToast("success", okMsg);
      closeForm();
      await load();
    } catch (e) {
      formError = errText(e);
    } finally {
      busy = false;
    }
  }

  const normUrl = (u: string) => {
    const s = u.trim();
    return /^[a-z]+:\/\//i.test(s) ? s : `http://${s}`;
  };

  // ---- Jellyfin ----
  function connectJf() {
    void run(() => api.jfConnect(normUrl(jfServer), jfUser.trim(), jfPass), $t("video.sources.connected_ok") as string);
  }

  async function startQc() {
    if (!jfServer.trim()) {
      formError = $t("video.sources.jf_server_first") as string;
      return;
    }
    busy = true;
    formError = null;
    const server = normUrl(jfServer);
    try {
      qc = await api.jfQuickConnectStart(server);
      const started = Date.now();
      qcTimer = setInterval(async () => {
        if (!qc) return;
        if (Date.now() - started > 5 * 60_000) {
          stopQc();
          formError = $t("video.sources.qc_expired") as string;
          return;
        }
        try {
          const done = await api.jfQuickConnectPoll(server, qc.secret);
          if (done) {
            stopQc();
            showToast("success", $t("video.sources.connected_ok") as string);
            closeForm();
            await load();
          }
        } catch (e) {
          stopQc();
          formError = errText(e);
        }
      }, 3000);
    } catch (e) {
      formError = errText(e);
    } finally {
      busy = false;
    }
  }

  function stopQc() {
    if (qcTimer) clearInterval(qcTimer);
    qcTimer = null;
    qc = null;
  }

  // ---- IPTV ----
  async function pickM3u() {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const f = await open({ multiple: false, directory: false, filters: [{ name: "M3U", extensions: ["m3u", "m3u8", "txt"] }] }).catch(() => null);
    if (f && !Array.isArray(f)) {
      m3uFile = f;
      if (!m3uName) m3uName = f.split(/[\\/]/).pop()?.replace(/\.[^.]+$/, "") ?? "";
    }
  }

  function addM3u() {
    const url = m3uMode === "file" ? m3uFile : normUrl(m3uUrl);
    void run(async () => {
      const s = await api.iptvAdd(m3uName.trim() || ($t("video.sources.m3u_title") as string), url, m3uEpg.trim() ? normUrl(m3uEpg) : null);
      void api.iptvRefresh(s.id).catch(() => {});
    }, $t("video.sources.iptv_added") as string);
  }

  function addXtream() {
    void run(async () => {
      const s = await api.iptvAddXtream(xtName.trim() || "Xtream", normUrl(xtServer), xtUser.trim(), xtPass);
      void api.iptvRefresh(s.id).catch(() => {});
    }, $t("video.sources.iptv_added") as string);
  }

  // ---- add-ons ----
  function addAddon() {
    void run(() => api.addonInstall(addonUrl.trim()), $t("video.sources.addon_added") as string);
  }

  // ---- AniList ----
  function saveAni() {
    void run(async () => {
      await api.anilistSetToken(aniToken.trim());
      const v = await api.anilistViewer();
      if (!v) throw new Error($t("video.sources.ani_bad_token") as string);
      aniToken = "";
    }, $t("video.sources.ani_connected") as string);
  }

  async function disconnectAni() {
    try {
      await api.anilistSetToken("");
      viewer = null;
      confirmId = null;
      showToast("success", $t("video.sources.ani_disconnected") as string);
    } catch (e) {
      showToast("error", errText(e));
    }
  }

  async function openLink(url: string) {
    try {
      const { open } = await import("@tauri-apps/plugin-shell");
      await open(url);
    } catch (e) {
      showToast("error", errText(e));
    }
  }

  // ---- list ----
  async function toggle(s: SourceInfo) {
    const next = !s.enabled;
    s.enabled = next;
    try {
      await api.sourceToggle(s.id, next);
      await load();
    } catch (e) {
      s.enabled = !next;
      showToast("error", errText(e));
    }
  }

  async function remove(s: SourceInfo) {
    try {
      await api.sourceRemove(s.id);
      confirmId = null;
      showToast("success", $t("video.sources.removed", { name: s.name }) as string);
      await load();
    } catch (e) {
      showToast("error", errText(e));
    }
  }

  async function removeAddon(a: AddonInfo) {
    try {
      await api.addonRemove(a.id);
      confirmId = null;
      showToast("success", $t("video.sources.removed", { name: a.name }) as string);
      await load();
    } catch (e) {
      showToast("error", errText(e));
    }
  }

  async function refreshIptv(s: SourceInfo) {
    refreshing = s.id;
    try {
      await api.iptvRefresh(s.id);
      showToast("success", $t("video.live.refreshed") as string);
      await load();
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      refreshing = null;
    }
  }

  function kindIcon(k: SourceKind): string {
    return k === "jellyfin" ? "server" : k === "iptv" ? "live" : "addons";
  }

  function statusTag(s: SourceInfo): { cls: string; label: string } {
    if (!s.enabled || s.status === "disabled") return { cls: "", label: $t("video.sources.status_disabled") as string };
    if (s.status === "down") return { cls: "tag-danger", label: $t("video.sources.status_down") as string };
    if (s.status === "slow") return { cls: "tag-warning", label: $t("video.sources.status_slow") as string };
    return { cls: "tag-success", label: $t("video.sources.status_ok") as string };
  }

  function detail(s: SourceInfo): string {
    const x = s.extra ?? {};
    if (s.kind === "jellyfin") return [x.server_name, x.user].filter(Boolean).join(" · ") || s.url;
    if (s.kind === "iptv" && typeof x.channels === "number") return $t("video.sources.channels", { count: x.channels }) as string;
    return s.url;
  }

  function openSource(s: SourceInfo) {
    if (s.kind === "jellyfin") goRoot({ name: "jellyfin", source: s.id });
    else if (s.kind === "iptv") goRoot({ name: "live" });
    else goRoot({ name: "addons" });
  }

  function when(ts: number): string {
    return ts ? new Date(ts).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" }) : "";
  }
</script>

<div class="page">
  <Head title={$t("video.sources.title") as string} subtitle={$t("video.sources.subtitle") as string} />

  <div class="notice notice-info">
    <VIcon name="shield" size={16} />
    <div class="notice-body">
      <span class="notice-title">{$t("video.sources.bring_your_own_title")}</span>
      <span class="notice-text">{$t("video.sources.bring_your_own_body")}</span>
    </div>
  </div>

  <div class="cards">
    {#each cards as c (c.id)}
      {@const connected = c.id === "anilist" ? !!viewer : false}
      <article class="card">
        <img src="/icons/video/{c.icon}-64.webp" alt="" width="44" height="44" />
        <h3>{c.title}</h3>
        <p>{c.body}</p>
        <small class="sends"><VIcon name="info" size={11} /> {c.sends}</small>
        <div class="c-foot">
          {#if connected}
            <span class="tag tag-success"><VIcon name="check" size={11} /> {$t("video.sources.connected_as", { name: viewer?.name ?? "" })}</span>
          {:else}
            <button class="btn btn-sm btn-tinted" onclick={() => openForm(c.id)}>
              {c.id === "addon" || c.id === "m3u" || c.id === "xtream" ? $t("video.sources.add") : $t("video.sources.connect")}
              <VIcon name="chevron-right" size={12} />
            </button>
          {/if}
        </div>
      </article>
    {/each}
  </div>

  <section class="mine">
    <h2>{$t("video.sources.yours")}</h2>
    {#if error}
      <div class="notice notice-danger"><VIcon name="alert" size={16} /><div class="notice-body">{error}</div></div>
    {/if}
    {#if !sources}
      <Skel kind="list" count={3} />
    {:else if !sources.length && !looseAddons.length && !viewer}
      <p class="empty">{$t("video.sources.none_yet")}</p>
    {:else}
      <ul class="rows" role="list">
        {#each sources as s (s.id)}
          {@const st = statusTag(s)}
          {@const key = `s${s.id}`}
          <li class="row" class:off={!s.enabled}>
            <div class="r-main">
              <img src="/icons/video/{kindIcon(s.kind)}-64.webp" alt="" width="32" height="32" />
              <button class="r-tx" onclick={() => openSource(s)} disabled={!s.enabled}>
                <b>{s.name}</b>
                <small title={s.url}>{detail(s)}</small>
              </button>
              <span class="tag {st.cls}">{st.label}</span>
              {#if s.kind === "iptv"}
                <button class="btn btn-icon btn-sm btn-ghost" onclick={() => void refreshIptv(s)} disabled={refreshing === s.id || !s.enabled} aria-label={$t("video.live.refresh") as string} title={$t("video.live.refresh") as string}>
                  <VIcon name="refresh" size={14} spin={refreshing === s.id} />
                </button>
              {/if}
              <button class="toggle toggle-sm" role="switch" aria-checked={s.enabled} aria-label={$t("video.sources.enable", { name: s.name }) as string} onclick={() => void toggle(s)}><span class="toggle-knob"></span></button>
              <button class="btn btn-icon btn-sm btn-ghost" onclick={() => (confirmId = confirmId === key ? null : key)} aria-label={$t("video.sources.remove") as string} title={$t("video.sources.remove") as string}>
                <VIcon name="trash" size={14} />
              </button>
            </div>
            {#if s.enabled && (s.status === "down" || s.status === "slow")}
              <p class="r-err" class:warn={s.status === "slow"}>
                {s.status === "down" ? $t("video.sources.down_since", { time: when(s.updated_at) }) : $t("video.sources.slow_hint")}
                {#if s.last_error}<span class="r-msg">{s.last_error}</span>{/if}
              </p>
            {/if}
            {#if confirmId === key}
              <div class="confirm">
                <span>{$t("video.sources.remove_confirm", { name: s.name })}</span>
                <button class="btn btn-sm btn-destructive" onclick={() => void remove(s)}>{$t("video.sources.remove")}</button>
                <button class="btn btn-sm" onclick={() => (confirmId = null)}>{$t("video.common.cancel")}</button>
              </div>
            {/if}
          </li>
        {/each}
        {#each looseAddons as a (a.id)}
          {@const key = `a${a.id}`}
          <li class="row">
            <div class="r-main">
              <img src="/icons/video/addons-64.webp" alt="" width="32" height="32" />
              <button class="r-tx" onclick={() => goRoot({ name: "addons", addon: a.id })}>
                <b>{a.name} <small class="ver">{a.version}</small></b>
                <small title={a.manifest_url}>{a.description || a.manifest_url}</small>
              </button>
              <span class="tag tag-success">{$t("video.sources.status_ok")}</span>
              <button class="btn btn-icon btn-sm btn-ghost" onclick={() => (confirmId = confirmId === key ? null : key)} aria-label={$t("video.sources.remove") as string}>
                <VIcon name="trash" size={14} />
              </button>
            </div>
            {#if confirmId === key}
              <div class="confirm">
                <span>{$t("video.sources.remove_confirm", { name: a.name })}</span>
                <button class="btn btn-sm btn-destructive" onclick={() => void removeAddon(a)}>{$t("video.sources.remove")}</button>
                <button class="btn btn-sm" onclick={() => (confirmId = null)}>{$t("video.common.cancel")}</button>
              </div>
            {/if}
          </li>
        {/each}
        {#if viewer}
          <li class="row">
            <div class="r-main">
              <img src="/icons/video/anime-64.webp" alt="" width="32" height="32" />
              <button class="r-tx" onclick={() => goRoot({ name: "anime" })}>
                <b>AniList</b>
                <small>{$t("video.sources.connected_as", { name: viewer.name })}</small>
              </button>
              <span class="tag tag-success">{$t("video.sources.status_ok")}</span>
              <button class="btn btn-sm btn-ghost" onclick={() => (confirmId = confirmId === "ani" ? null : "ani")}>{$t("video.sources.disconnect")}</button>
            </div>
            {#if confirmId === "ani"}
              <div class="confirm">
                <span>{$t("video.sources.ani_disconnect_confirm")}</span>
                <button class="btn btn-sm btn-destructive" onclick={() => void disconnectAni()}>{$t("video.sources.disconnect")}</button>
                <button class="btn btn-sm" onclick={() => (confirmId = null)}>{$t("video.common.cancel")}</button>
              </div>
            {/if}
          </li>
        {/if}
      </ul>
    {/if}
  </section>
</div>

{#if form}
  <Modal title={cards.find((c) => c.id === form)?.title ?? ""} width={460} onclose={closeForm}>
    <div class="dialog-body fm">
      {#if form === "jellyfin"}
        {#if qc}
          <div class="qc">
            <p>{$t("video.sources.qc_enter")}</p>
            <div class="qc-code" aria-live="polite">{qc.code}</div>
            <p class="hint">{$t("video.sources.qc_where")}</p>
            <div class="qc-wait"><span class="spinner"></span> {$t("video.sources.qc_waiting")}</div>
          </div>
          <div class="dialog-actions">
            <button class="btn" onclick={stopQc}>{$t("video.common.cancel")}</button>
          </div>
        {:else}
          <form
            onsubmit={(e) => {
              e.preventDefault();
              connectJf();
            }}
          >
            <label class="field">
              <span class="field-label">{$t("video.sources.jf_server")}</span>
              <input class="input" type="url" inputmode="url" placeholder="http://192.168.0.10:8096" bind:value={jfServer} required autocomplete="url" />
            </label>
            <label class="field">
              <span class="field-label">{$t("video.sources.username")}</span>
              <input class="input" bind:value={jfUser} required autocomplete="username" />
            </label>
            <label class="field">
              <span class="field-label">{$t("video.sources.password")}</span>
              <input class="input" type="password" bind:value={jfPass} autocomplete="current-password" />
            </label>
            {#if formError}<p class="field-error">{formError}</p>{/if}
            <div class="dialog-actions">
              <button type="button" class="btn" onclick={closeForm}>{$t("video.common.cancel")}</button>
              <button type="submit" class="btn btn-primary" disabled={busy || !jfServer.trim() || !jfUser.trim()}>
                {#if busy}<span class="spinner"></span>{/if}
                {$t("video.sources.sign_in")}
              </button>
            </div>
          </form>
          <div class="or"><span>{$t("video.sources.or")}</span></div>
          <button class="btn btn-block" onclick={() => void startQc()} disabled={busy}>
            <VIcon name="key" size={14} /> {$t("video.sources.qc_use")}
          </button>
          <p class="hint">{$t("video.sources.qc_hint")}</p>
        {/if}
      {:else if form === "m3u"}
        <form
          onsubmit={(e) => {
            e.preventDefault();
            addM3u();
          }}
        >
          <label class="field">
            <span class="field-label">{$t("video.sources.name")}</span>
            <input class="input" bind:value={m3uName} placeholder={$t("video.sources.m3u_name_ph") as string} />
          </label>
          <div class="segmented" role="tablist" aria-label={$t("video.sources.m3u_where") as string}>
            <button type="button" class="segmented-btn" role="tab" aria-selected={m3uMode === "url"} onclick={() => (m3uMode = "url")}><VIcon name="link" size={12} /> {$t("video.sources.from_url")}</button>
            <button type="button" class="segmented-btn" role="tab" aria-selected={m3uMode === "file"} onclick={() => (m3uMode = "file")}><VIcon name="file" size={12} /> {$t("video.sources.from_file")}</button>
          </div>
          {#if m3uMode === "url"}
            <label class="field">
              <span class="field-label">{$t("video.sources.m3u_url")}</span>
              <input class="input" type="url" inputmode="url" placeholder="https://…/playlist.m3u" bind:value={m3uUrl} />
            </label>
          {:else}
            <div class="field">
              <span class="field-label">{$t("video.sources.m3u_file")}</span>
              <div class="filepick">
                <button type="button" class="btn btn-sm" onclick={() => void pickM3u()}><VIcon name="upload" size={12} /> {$t("video.sources.choose_file")}</button>
                <small title={m3uFile}>{m3uFile || $t("video.sources.no_file")}</small>
              </div>
            </div>
          {/if}
          <label class="field">
            <span class="field-label">{$t("video.sources.epg_url")}</span>
            <input class="input" type="url" inputmode="url" placeholder="https://…/guide.xml.gz" bind:value={m3uEpg} />
            <span class="field-hint">{$t("video.sources.epg_hint")}</span>
          </label>
          {#if formError}<p class="field-error">{formError}</p>{/if}
          <div class="dialog-actions">
            <button type="button" class="btn" onclick={closeForm}>{$t("video.common.cancel")}</button>
            <button type="submit" class="btn btn-primary" disabled={busy || (m3uMode === "url" ? !m3uUrl.trim() : !m3uFile)}>
              {#if busy}<span class="spinner"></span>{/if}
              {$t("video.sources.add")}
            </button>
          </div>
        </form>
      {:else if form === "xtream"}
        <form
          onsubmit={(e) => {
            e.preventDefault();
            addXtream();
          }}
        >
          <label class="field">
            <span class="field-label">{$t("video.sources.name")}</span>
            <input class="input" bind:value={xtName} placeholder="Xtream" />
          </label>
          <label class="field">
            <span class="field-label">{$t("video.sources.xt_server")}</span>
            <input class="input" type="url" inputmode="url" placeholder="http://provider.example:8080" bind:value={xtServer} required />
          </label>
          <label class="field">
            <span class="field-label">{$t("video.sources.username")}</span>
            <input class="input" bind:value={xtUser} required autocomplete="username" />
          </label>
          <label class="field">
            <span class="field-label">{$t("video.sources.password")}</span>
            <input class="input" type="password" bind:value={xtPass} required autocomplete="current-password" />
          </label>
          <p class="hint">{$t("video.sources.xt_hint")}</p>
          {#if formError}<p class="field-error">{formError}</p>{/if}
          <div class="dialog-actions">
            <button type="button" class="btn" onclick={closeForm}>{$t("video.common.cancel")}</button>
            <button type="submit" class="btn btn-primary" disabled={busy || !xtServer.trim() || !xtUser.trim() || !xtPass}>
              {#if busy}<span class="spinner"></span>{/if}
              {$t("video.sources.add")}
            </button>
          </div>
        </form>
      {:else if form === "addon"}
        <form
          onsubmit={(e) => {
            e.preventDefault();
            addAddon();
          }}
        >
          <label class="field">
            <span class="field-label">{$t("video.sources.manifest_url")}</span>
            <input class="input" type="url" inputmode="url" placeholder="https://…/manifest.json" bind:value={addonUrl} required />
            <span class="field-hint">{$t("video.sources.manifest_hint")}</span>
          </label>
          <div class="notice notice-warning">
            <VIcon name="shield" size={16} />
            <div class="notice-body">
              <span class="notice-title">{$t("video.sources.addon_policy_title")}</span>
              <span class="notice-text">{$t("video.sources.addon_policy_body")}</span>
            </div>
          </div>
          {#if formError}<p class="field-error">{formError}</p>{/if}
          <div class="dialog-actions">
            <button type="button" class="btn" onclick={closeForm}>{$t("video.common.cancel")}</button>
            <button type="submit" class="btn btn-primary" disabled={busy || !addonUrl.trim()}>
              {#if busy}<span class="spinner"></span>{/if}
              {$t("video.sources.install")}
            </button>
          </div>
        </form>
      {:else if form === "anilist"}
        <form
          onsubmit={(e) => {
            e.preventDefault();
            saveAni();
          }}
        >
          <ol class="steps">
            <li>
              {$t("video.sources.ani_step1")}
              <button type="button" class="linkbtn" onclick={() => void openLink(ANILIST_TOKEN_URL)}>{$t("video.sources.ani_get_token")} <VIcon name="external" size={11} /></button>
            </li>
            <li>{$t("video.sources.ani_step2")}</li>
          </ol>
          <label class="field">
            <span class="field-label">{$t("video.sources.ani_token")}</span>
            <textarea class="input" rows="4" bind:value={aniToken} spellcheck="false" autocomplete="off" required></textarea>
          </label>
          {#if formError}<p class="field-error">{formError}</p>{/if}
          <div class="dialog-actions">
            <button type="button" class="btn" onclick={closeForm}>{$t("video.common.cancel")}</button>
            <button type="submit" class="btn btn-primary" disabled={busy || !aniToken.trim()}>
              {#if busy}<span class="spinner"></span>{/if}
              {$t("video.sources.connect")}
            </button>
          </div>
        </form>
      {/if}
    </div>
  </Modal>
{/if}

<style>
  .page {
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
    padding-bottom: var(--space-7);
  }
  .cards {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(230px, 1fr));
    gap: var(--space-3);
  }
  .card {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-4);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }
  .card img {
    width: 44px;
    height: 44px;
  }
  .card h3 {
    margin: var(--space-1) 0 0;
    font-size: var(--text-md);
    font-weight: 650;
  }
  .card p {
    margin: 0;
    flex: 1;
    color: var(--text-dim);
    font-size: var(--text-sm);
    line-height: 1.45;
  }
  .sends {
    display: flex;
    gap: 4px;
    align-items: flex-start;
    color: var(--text-faint);
    font-size: var(--text-xs);
    line-height: 1.4;
  }
  .sends :global(svg) {
    margin-top: 2px;
  }
  .c-foot {
    display: flex;
    margin-top: var(--space-1);
  }
  .mine {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  h2 {
    margin: 0;
    font-size: var(--text-xl);
    font-weight: 700;
  }
  .empty {
    margin: 0;
    color: var(--text-dim);
  }
  .rows {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }
  .row {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3);
  }
  .row + .row {
    border-top: var(--hairline) solid var(--separator);
  }
  .row.off .r-tx {
    opacity: 0.55;
  }
  .r-main {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }
  .r-main img {
    width: 32px;
    height: 32px;
  }
  .r-tx {
    all: unset;
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    cursor: pointer;
  }
  .r-tx:disabled {
    cursor: default;
  }
  .r-tx:focus-visible {
    outline: var(--focus-ring);
    border-radius: var(--radius-sm);
  }
  .r-tx b {
    font-weight: 600;
  }
  .r-tx small {
    color: var(--text-dim);
    font-size: var(--text-sm);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .ver {
    font-weight: 400;
    font-size: var(--text-xs) !important;
  }
  .r-err {
    margin: 0 0 0 44px;
    font-size: var(--text-sm);
    color: var(--danger);
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .r-err.warn {
    color: var(--warning);
  }
  .r-msg {
    color: var(--text-dim);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    word-break: break-word;
  }
  .confirm {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
    margin-left: 44px;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: color-mix(in srgb, var(--danger) 10%, transparent);
    font-size: var(--text-sm);
  }
  .confirm span {
    flex: 1;
    min-width: 200px;
  }
  .fm form {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .or {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin: var(--space-3) 0;
    color: var(--text-dim);
    font-size: var(--text-xs);
  }
  .or::before,
  .or::after {
    content: "";
    flex: 1;
    height: var(--hairline);
    background: var(--separator);
  }
  .hint {
    margin: var(--space-2) 0 0;
    color: var(--text-dim);
    font-size: var(--text-sm);
    line-height: 1.45;
  }
  .qc {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-2);
    text-align: center;
  }
  .qc p {
    margin: 0;
  }
  .qc-code {
    font-family: var(--font-mono);
    font-size: 40px;
    font-weight: 700;
    letter-spacing: 0.2em;
    padding: var(--space-3) var(--space-5);
    border-radius: var(--radius-lg);
    background: var(--fill-1);
    user-select: all;
  }
  .qc-wait {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    color: var(--text-dim);
    font-size: var(--text-sm);
  }
  .filepick {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }
  .filepick small {
    color: var(--text-dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .steps {
    margin: 0;
    padding-left: 1.2em;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    font-size: var(--text-sm);
    line-height: 1.45;
  }
  .linkbtn {
    all: unset;
    display: inline-flex;
    align-items: center;
    gap: 3px;
    color: var(--accent-text);
    font-weight: 600;
    cursor: pointer;
  }
  .linkbtn:focus-visible {
    outline: var(--focus-ring);
  }
</style>
