<script lang="ts">
  // Subscribe / Subscribed (21st 818: the label slides, the bell swings).
  // With an account it goes to YouTube; without one it is a local
  // subscription (feed built from RSS), with its own notify toggle.
  import { t } from "$lib/i18n";
  import * as api from "$lib/tube/api";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import TIcon from "../TIcon.svelte";
  import Popup from "./Popup.svelte";
  import { dropLocalSub, ensureSession, localSub, loggedIn, putLocalSub, sessionReady } from "./session.svelte";
  import { errText } from "./util";

  let {
    channel,
    subscribed = null,
    size = "m",
  }: {
    channel: { id: string; name: string; avatar?: string | null };
    subscribed?: boolean | null;
    size?: "s" | "m";
  } = $props();

  let override = $state<boolean | null>(null);
  let busy = $state(false);
  let menu = $state(false);
  let confirm = $state(false);
  let justNow = $state(false);
  let btn = $state<HTMLButtonElement>();

  $effect(() => {
    void ensureSession();
  });

  let local = $derived(sessionReady() && !loggedIn());
  let isSub = $derived(override ?? (local ? !!localSub(channel.id) : !!subscribed));
  let notify = $derived(!!localSub(channel.id)?.notify);

  async function subscribe() {
    if (busy) return;
    busy = true;
    override = true;
    justNow = true;
    try {
      await ensureSession();
      if (loggedIn()) {
        await api.action("subscribe", { channel_id: channel.id });
      } else {
        putLocalSub(await api.subsAdd(channel.id, channel.name, channel.avatar ?? null));
      }
    } catch (e) {
      override = false;
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }

  async function unsubscribe() {
    confirm = false;
    menu = false;
    busy = true;
    override = false;
    justNow = false;
    try {
      if (loggedIn()) {
        await api.action("unsubscribe", { channel_id: channel.id });
      } else {
        await api.subsRemove(channel.id);
        dropLocalSub(channel.id);
      }
    } catch (e) {
      override = true;
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }

  async function toggleNotify() {
    const s = localSub(channel.id);
    if (!s) return;
    const next = !s.notify;
    putLocalSub({ ...s, notify: next });
    try {
      await api.subsUpdate(channel.id, next, null);
    } catch (e) {
      putLocalSub({ ...s, notify: !next });
      showToast("error", errText(e));
    }
  }
</script>

{#if isSub}
  <button
    bind:this={btn}
    class="yt-btn sub on {size === 's' ? 'yt-btn--s' : ''}"
    class:just={justNow}
    aria-haspopup="menu"
    aria-expanded={menu}
    disabled={busy}
    onclick={() => (menu = !menu)}
  >
    <span class="bell"><TIcon name={notify ? "bell-fill" : "bell"} size={size === "s" ? 18 : 22} /></span>
    <span class="label">{$t("tube.channel.subscribed")}</span>
    <TIcon name="chevron-down" size={18} />
  </button>
{:else}
  <button class="yt-btn yt-btn--inverted sub {size === 's' ? 'yt-btn--s' : ''}" disabled={busy} onclick={subscribe}>
    <span class="label">{$t("tube.channel.subscribe")}</span>
  </button>
{/if}

{#if menu}
  <Popup anchor={btn ?? null} align="start" onclose={() => (menu = false)} label={$t("tube.channel.subscribed") as string}>
    {#if local}
      <button class="yt-menu-item" role="menuitemcheckbox" aria-checked={notify} onclick={toggleNotify}>
        <TIcon name={notify ? "bell-fill" : "bell"} />{$t(notify ? "tube.channel.notify_on" : "tube.channel.notify_off")}
      </button>
      <p class="hint">{$t("tube.channel.local_hint")}</p>
    {/if}
    <button class="yt-menu-item" role="menuitem" onclick={() => { menu = false; confirm = true; }}>
      <TIcon name="user" />{$t("tube.channel.unsubscribe")}
    </button>
  </Popup>
{/if}

{#if confirm}
  <Popup modal class="yt-menu confirm" onclose={() => (confirm = false)} label={$t("tube.channel.unsubscribe") as string} width={340}>
    <p class="q">{$t("tube.channel.unsubscribe_confirm", { name: channel.name })}</p>
    <div class="row">
      <button class="yt-btn yt-btn--ghost" onclick={() => (confirm = false)}>{$t("tube.common.cancel")}</button>
      <button class="yt-btn yt-btn--ghost link" onclick={unsubscribe}>{$t("tube.channel.unsubscribe")}</button>
    </div>
  </Popup>
{/if}

<style>
  .sub {
    overflow: hidden;
  }
  .label {
    display: inline-block;
  }
  .sub:not(.on) .label {
    animation: none;
  }
  .just .label {
    animation: slide-in 260ms var(--yt-ease-standard);
  }
  .just .bell {
    animation: swing 900ms var(--yt-ease-standard);
    transform-origin: 50% 10%;
  }
  .bell {
    display: inline-flex;
  }
  .hint {
    margin: 0;
    padding: 0 16px 8px 56px;
    font: var(--yt-type-caption);
    color: var(--yt-text-2);
  }
  .q {
    margin: 8px 24px 16px;
    font: var(--yt-type-meta);
    font-size: 16px;
    line-height: 22px;
  }
  .row {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    padding: 0 12px 4px;
  }
  .link {
    color: var(--yt-link);
  }
  @keyframes slide-in {
    from {
      transform: translateY(-120%);
      opacity: 0;
    }
  }
  @keyframes swing {
    0% { transform: rotate(0); }
    20% { transform: rotate(18deg); }
    40% { transform: rotate(-14deg); }
    60% { transform: rotate(8deg); }
    80% { transform: rotate(-4deg); }
    100% { transform: rotate(0); }
  }
  @media (prefers-reduced-motion: reduce) {
    .just .label,
    .just .bell {
      animation: none;
    }
  }
</style>
