<script lang="ts">
  // "You" (spec §2.11): header with avatar 120 and name 36/700, then rows:
  // history, playlists, watch later, liked. Without an account: an invite to
  // sign in plus what is kept locally (history, watch later, local subs).
  import { t } from "$lib/i18n";
  import * as api from "$lib/tube/api";
  import { bestAvatar } from "$lib/tube/format";
  import { go, type Route } from "$lib/tube/ui.svelte";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import TIcon from "../../TIcon.svelte";
  import Shelf from "../../cards/Shelf.svelte";
  import State from "../../cards/State.svelte";
  import UndoPill from "../../cards/UndoPill.svelte";
  import { Pager } from "../../cards/pager.svelte";
  import { account, ensureSession, loggedIn, login as signIn, sessionReady } from "../../cards/session.svelte";
  import { channelHref, errText, openChannel } from "../../cards/util";

  let { route: _route }: { route: Extract<Route, { name: "library" }> } = $props();

  const pager = new Pager("feed", () => api.library());
  let signingIn = $state(false);

  $effect(() => {
    void (async () => {
      await ensureSession();
      await pager.load();
    })();
  });

  async function login() {
    signingIn = true;
    try {
      await signIn();
      await pager.load();
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      signingIn = false;
    }
  }

  let acct = $derived(account());
  let shelves = $derived(pager.shelves.filter((s) => s.items.length > 0));

  // Local shelves carry no "more" nav; point the known ones at their screens.
  function withMore(s: (typeof shelves)[number]) {
    if (s.more) return s;
    const id = s.id.toLowerCase();
    if (id.includes("history")) return { ...s, more: { browse_id: "FEhistory" } };
    if (id.includes("watch_later") || id === "wl") return { ...s, more: { browse_id: "VLWL" } };
    if (id.includes("liked") || id === "ll") return { ...s, more: { browse_id: "VLLL" } };
    if (id.includes("sub")) return { ...s, more: { browse_id: "FEsubscriptions" } };
    return s;
  }
</script>

<div class="you" data-agent-readiness={pager.loading ? "loading" : pager.error ? "error" : "ready"} data-agent-surface="omnitube-you">
  <div class="inner">
    {#if !sessionReady()}
      <div class="head">
        <div class="yt-skel av-sk"></div>
        <div class="ht">
          <div class="yt-skel" style="height: 36px; width: 240px"></div>
          <div class="yt-skel" style="height: 16px; width: 160px"></div>
        </div>
      </div>
    {:else if loggedIn() && acct}
      <header class="head">
        {#if acct.avatar}
          <img class="yt-avatar av" src={bestAvatar(acct.avatar, 120)} width="120" height="120" alt="" referrerpolicy="no-referrer" />
        {:else}
          <span class="yt-avatar av ph">{(acct.name ?? "?").slice(0, 1)}</span>
        {/if}
        <div class="ht">
          <h1>{acct.name}</h1>
          <div class="facts">
            {#if acct.handle}<span>{acct.handle}</span>{/if}
            {#if acct.channel_id}
              <span aria-hidden="true">•</span>
              <a href={channelHref(acct.channel_id)} onclick={(e) => { e.preventDefault(); openChannel(acct!.channel_id!); }}>{$t("tube.you.view_channel")}</a>
            {/if}
          </div>
          <div class="acts">
            <button class="yt-btn yt-btn--s" onclick={() => go({ name: "settings", section: "account" })}><TIcon name="user" size={18} />{$t("tube.you.account")}</button>
            <button class="yt-btn yt-btn--s" onclick={() => go({ name: "import" })}><TIcon name="import" size={18} />{$t("tube.you.import")}</button>
          </div>
        </div>
      </header>
    {:else}
      <section class="invite">
        <img src="/icons/tube/library-128.webp" width="88" height="88" alt="" />
        <div class="inv-text">
          <h1>{$t("tube.you.invite_title")}</h1>
          <p>{$t("tube.you.invite_text")}</p>
          <div class="acts">
            <button class="yt-btn yt-btn--outline" disabled={signingIn} onclick={login}><TIcon name="login" size={20} />{$t("tube.common.sign_in")}</button>
            <button class="yt-btn" onclick={() => go({ name: "import" })}><TIcon name="import" size={20} />{$t("tube.you.import")}</button>
          </div>
        </div>
      </section>
    {/if}

    {#if pager.loading && !pager.page}
      {#each Array(3) as _, i (i)}
        <div class="sk-shelf">
          <div class="yt-skel" style="height: 28px; width: 180px"></div>
          <div class="sk-row">
            {#each Array(4) as __, j (j)}<div class="yt-skel sk-card"></div>{/each}
          </div>
        </div>
      {/each}
    {:else if pager.error && !pager.page}
      <State icon="library" error={pager.error} onretry={() => pager.load()} />
    {:else if !shelves.length}
      <State icon="library" title={$t("tube.you.empty_title") as string} text={$t("tube.you.empty_text") as string}>
        <button class="yt-btn yt-btn--outline" onclick={() => go({ name: "home" })}>{$t("tube.you.go_home")}</button>
      </State>
    {:else}
      <div class="shelves">
        {#each shelves as s, i (s.id)}
          <Shelf shelf={{ ...withMore(s), kind: s.kind === "grid" || s.kind === "list" ? "row" : s.kind }} first={i === 0} feedbackActions={false} />
        {/each}
      </div>
    {/if}
  </div>
  <UndoPill />
</div>

<style>
  .you {
    padding: 0 var(--yt-grid-gutter) 24px;
  }
  .inner {
    max-width: 1284px;
    margin: 0 auto;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 16px;
    padding: 24px 0 32px;
  }
  .av,
  .av-sk {
    flex: none;
    width: 120px;
    height: 120px;
    border-radius: 50%;
  }
  .ph {
    display: grid;
    place-items: center;
    font: 500 48px/1 var(--yt-font);
    color: var(--yt-text-2);
    text-transform: uppercase;
  }
  .ht {
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-width: 0;
  }
  h1 {
    margin: 0;
    font: var(--yt-type-display);
  }
  .facts {
    display: flex;
    gap: 6px;
    font: var(--yt-type-meta);
    color: var(--yt-text-2);
  }
  .facts a {
    color: var(--yt-text-2);
    text-decoration: none;
  }
  .facts a:hover {
    color: var(--yt-text);
  }
  .acts {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    margin-top: 4px;
  }
  .invite {
    display: flex;
    align-items: center;
    gap: 24px;
    margin: 24px 0 32px;
    padding: 24px;
    border-radius: 16px;
    background: var(--yt-tonal);
  }
  .invite h1 {
    font: var(--yt-type-headline-m);
  }
  .inv-text {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .invite p {
    margin: 0;
    max-width: 560px;
    color: var(--yt-text-2);
  }
  .shelves {
    display: flex;
    flex-direction: column;
    gap: 32px;
  }
  .sk-shelf {
    display: flex;
    flex-direction: column;
    gap: 16px;
    margin-bottom: 32px;
  }
  .sk-row {
    display: flex;
    gap: 16px;
    overflow: hidden;
  }
  .sk-card {
    flex: none;
    width: 290px;
    aspect-ratio: 16 / 9;
    border-radius: 12px;
  }
  @container omnitube (max-width: 640px) {
    .you {
      padding: 0 12px 24px;
    }
    .av,
    .av-sk {
      width: 72px;
      height: 72px;
    }
    h1 {
      font: var(--yt-type-headline-m);
    }
    .invite {
      flex-direction: column;
      align-items: flex-start;
    }
  }
</style>
