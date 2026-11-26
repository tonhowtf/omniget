<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { open as shellOpen } from "@tauri-apps/plugin-shell";
  import { t, locale } from "$lib/i18n";
  import { getSettings } from "$lib/stores/settings-store.svelte";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import NavIcon from "#components/shell/NavIcon.svelte";
  import PostCard from "./PostCard.svelte";
  import SocialIcon from "./SocialIcon.svelte";
  import AudienceTab from "./AudienceTab.svelte";
  import AnalyticsTab from "./AnalyticsTab.svelte";
  import MonitorTab from "./MonitorTab.svelte";
  import ScheduleTab from "./ScheduleTab.svelte";
  import {
    ago,
    compact,
    errDetail,
    errKey,
    NET,
    SLUG,
    social,
    TITLE_KEY,
    tool,
    type Comment,
    type CodehubAuth,
    type Page,
    type Platform,
    type Post,
    type PostDetail,
    type Profile,
  } from "./api";

  let { platform }: { platform: Platform } = $props();

  type Tab = "feed" | "mentions" | "saved" | "search" | "assistant" | "me" | "audience" | "analytics" | "monitor" | "schedule";
  const ENGINE_TABS: Tab[] = ["audience", "analytics", "monitor", "schedule"];
  type AssistantMessage = { id: string; role: "user" | "assistant"; text: string; sources: string[] };
  type AssistantChat = { id: string; title: string; url: string };
  type View =
    | { kind: "tab" }
    | { kind: "profile"; handle: string }
    | { kind: "post"; post: Post };

  const TABS: Record<Platform, Tab[]> = {
    [NET.shortpost]: ["feed", "mentions", "saved", "search", "assistant", "me", "audience", "analytics", "monitor", "schedule"],
    [NET.photofeed]: ["feed", "saved", "search", "me", "audience", "analytics", "monitor"],
    [NET.clipfeed]: ["feed", "search", "me", "audience", "analytics", "monitor"],
    [NET.forums]: ["feed", "saved", "search", "me", "analytics", "monitor"],
    [NET.pinboard]: ["feed", "search", "me", "audience", "analytics", "monitor"],
    [NET.codehub]: ["feed", "search", "me", "audience", "analytics", "monitor"],
  };
  const FEEDS: Partial<Record<Platform, string[]>> = { [NET.forums]: ["home", "popular", "all"], [NET.codehub]: ["received", "trending"] };
  const KINDS: Partial<Record<Platform, { id: string; list: "posts" | "users" }[]>> = {
    [NET.forums]: [{ id: "posts", list: "posts" }, { id: "subreddits", list: "users" }, { id: "users", list: "users" }],
    [NET.pinboard]: [{ id: "pins", list: "posts" }, { id: "boards", list: "users" }, { id: "users", list: "users" }],
    [NET.codehub]: [{ id: "repositories", list: "posts" }, { id: "issues", list: "posts" }, { id: "users", list: "users" }],
  };
  const TAB_ICON: Record<Tab, string> = { feed: "home", mentions: "at", saved: "bookmark", search: "search", assistant: "spark", me: "user", audience: "users", analytics: "chart", monitor: "radar", schedule: "clock" };

  let loggedIn = $state<boolean | null>(null);
  let checking = $state(true);
  let waitingLogin = $state(false);
  let me = $state<Profile | null>(null);
  let tab = $state<Tab>("feed");
  let stack = $state<View[]>([]);
  let view = $derived<View>(stack[stack.length - 1] ?? { kind: "tab" });

  let posts = $state<Post[]>([]);
  let cursor = $state<string | null>(null);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let feedMode = $state<"for_you" | "following">("for_you");

  let profile = $state<Profile | null>(null);
  let detail = $state<PostDetail | null>(null);
  let replies = $state<(Post | Comment)[]>([]);
  let repliesCursor = $state<string | null>(null);

  let query = $state("");
  let searchKind = $state<"posts" | "users">("posts");
  let users = $state<Profile[]>([]);
  let hashtags = $state<{ name: string; posts?: number }[]>([]);


  let assistantMessages = $state<AssistantMessage[]>([]);
  let assistantChats = $state<AssistantChat[]>([]);
  let assistantConversation = $state<string | null>(null);
  let assistantDraft = $state("");
  let assistantBusy = $state(false);

  let draft = $state("");
  let replyDraft = $state("");
  let sending = $state(false);
  let acting = $state<Record<string, boolean>>({});
  let poll: ReturnType<typeof setInterval> | null = null;

  let netFeed = $state("");
  let netKind = $state("");
  let ghAuth = $state<CodehubAuth | null>(null);
  let ghToken = $state("");
  let ghUser = $state("");
  let ghBusy = $state(false);

  let tabs = $derived(TABS[platform]);
  let label = $derived($t(TITLE_KEY[platform]) as string);
  let newer = $derived(platform === NET.forums || platform === NET.pinboard || platform === NET.codehub);
  let feedPick = $derived(netFeed || FEEDS[platform]?.[0] || "");
  let kindPick = $derived(netKind || KINDS[platform]?.[0].id || "");
  let whoKey = $derived(platform === NET.shortpost ? "handle" : "username");
  let profileTool = $derived(tool(platform, newer ? "user" : "profile"));
  let postTool = $derived(platform === NET.pinboard ? "pin_pin" : platform === NET.codehub ? "gh_issues" : tool(platform, "post"));
  let canReply = $derived(platform !== NET.pinboard && (platform !== NET.codehub || me?.extra?.auth === "token"));

  function at(handle: string): string {
    if (handle.includes("/")) return handle;
    return platform === NET.forums ? `u/${handle}` : `@${handle}`;
  }

  function kindOf(p: Post): string {
    return String(p.extra?.kind ?? "");
  }

  function canFollow(p: Profile): boolean {
    if (!me || p.handle.toLowerCase() === me.handle.toLowerCase()) return false;
    if (platform === NET.forums) return p.handle.startsWith("r/");
    if (platform === NET.pinboard) return !p.handle.includes("/");
    if (platform === NET.codehub) return me.extra?.auth === "token";
    return true;
  }

  async function saveCodehub(auth: { token?: string; username?: string }) {
    ghBusy = true;
    error = null;
    try {
      ghAuth = await social.codehubConnect(auth);
      ghToken = "";
      await refreshStatus();
      if (loggedIn) await connect();
      else me = null;
    } catch (e) {
      fail(e);
    } finally {
      ghBusy = false;
    }
  }

  async function openCodehubSetup() {
    error = null;
    try {
      ghAuth = await social.codehubState();
      ghUser = ghAuth.username ?? "";
    } catch (e) {
      fail(e);
    }
    me = null;
  }

  function fail(e: unknown) {
    const key = errKey(e);
    if (key === "not_logged_in") loggedIn = false;
    error = `${$t(`social.errors.${key}`, { platform: label })}${key === "unknown" || key.startsWith("upstream") || key === "rejected" || (platform === NET.codehub && key !== "network") ? ` (${errDetail(e)})` : ""}`;
  }

  async function refreshStatus() {
    try {
      const s = await social.status();
      const mine = s.platforms.find((p) => p.platform === platform);
      loggedIn = mine?.logged_in ?? null;
    } catch {
      loggedIn = null;
    } finally {
      checking = false;
    }
  }

  async function connect() {
    checking = true;
    error = null;
    try {
      me = await social.call<Profile>(tool(platform, "me"));
      loggedIn = true;
      await loadTab();
    } catch (e) {
      fail(e);
    } finally {
      checking = false;
    }
  }

  async function signIn() {
    error = null;
    try {
      await social.login(platform);
      waitingLogin = true;
      stopPoll();
      poll = setInterval(async () => {
        await refreshStatus();
        if (loggedIn) {
          stopPoll();
          waitingLogin = false;
          await connect();
        }
      }, 3000);
    } catch (e) {
      fail(e);
    }
  }

  function stopPoll() {
    if (poll) clearInterval(poll);
    poll = null;
  }

  function listTool(): [string, Record<string, unknown>] | null {
    switch (tab) {
      case "feed":
        if (platform === NET.codehub && feedPick === "trending") return ["gh_trending", {}];
        return [tool(platform, "feed"), platform === NET.shortpost ? { mode: feedMode } : platform === NET.forums ? { feed: feedPick } : {}];
      case "mentions":
        return [tool(platform, "mentions"), {}];
      case "saved":
        return [platform === NET.shortpost ? "x_bookmarks" : tool(platform, "saved"), {}];
      case "me":
        return me ? [tool(platform, "user_posts"), { [whoKey]: me.handle }] : null;
      default:
        return null;
    }
  }

  async function loadTab(more = false) {
    error = null;
    if (ENGINE_TABS.includes(tab)) return;
    if (tab === "assistant") {
      if (!more) await loadAssistantChats();
      return;
    }
    if (tab === "search") {
      if (!more) {
        posts = [];
        users = [];
        hashtags = [];
        cursor = null;
      }
      if (query.trim()) await runSearch(more);
      return;
    }
    const spec = listTool();
    if (!spec) return;
    if (!more) {
      posts = [];
      cursor = null;
    }
    loading = true;
    try {
      const args = { ...spec[1], ...(more && cursor ? { cursor } : {}) };
      const page = await social.call<Page<Post>>(spec[0], args);
      posts = more ? dedupe([...posts, ...page.items]) : page.items;
      cursor = page.next_cursor;
    } catch (e) {
      fail(e);
    } finally {
      loading = false;
    }
  }

  async function loadAssistantChats() {
    try {
      assistantChats = (await social.call<Page<AssistantChat>>("x_assistant_chats", {})).items;
    } catch (e) {
      fail(e);
    }
  }

  async function openAssistantChat(id: string) {
    assistantConversation = id || null;
    assistantMessages = [];
    if (!id) return;
    assistantBusy = true;
    error = null;
    try {
      assistantMessages = (await social.call<Page<AssistantMessage>>("x_assistant_chat", { conversation: id })).items;
    } catch (e) {
      fail(e);
    } finally {
      assistantBusy = false;
    }
  }

  async function askAssistant(text = assistantDraft, post?: string) {
    const message = text.trim();
    if (!message || assistantBusy) return;
    assistantBusy = true;
    error = null;
    assistantDraft = "";
    assistantMessages = [...assistantMessages, { id: `local-${Date.now()}`, role: "user", text: message, sources: [] }];
    try {
      const r = await social.call<{ conversation_id: string; text: string; sources: string[] }>("x_assistant", {
        message,
        ...(assistantConversation ? { conversation: assistantConversation } : {}),
        ...(post ? { post } : {}),
      });
      const fresh = !assistantConversation;
      assistantConversation = r.conversation_id;
      assistantMessages = [...assistantMessages, { id: `assistant-${Date.now()}`, role: "assistant", text: r.text, sources: r.sources }];
      if (fresh) void loadAssistantChats();
    } catch (e) {
      assistantDraft = message;
      assistantMessages = assistantMessages.slice(0, -1);
      fail(e);
    } finally {
      assistantBusy = false;
    }
  }

  function askAssistantAbout(p: Post) {
    tab = "assistant";
    stack = [];
    assistantConversation = null;
    assistantMessages = [];
    void loadAssistantChats();
    void askAssistant($t("social.assistant.explain_post"), p.url || p.id);
  }

  function dedupe(list: Post[]): Post[] {
    const seen = new Set<string>();
    return list.filter((p) => (seen.has(p.id) ? false : (seen.add(p.id), true)));
  }

  async function runSearch(more = false) {
    loading = true;
    error = null;
    try {
      const kind = KINDS[platform]?.find((k) => k.id === kindPick);
      if (kind) {
        const r = await social.call<Page<unknown>>(tool(platform, "search"), { query, kind: kind.id, ...(more && cursor ? { cursor } : {}) });
        if (kind.list === "users") users = more ? [...users, ...(r.items as Profile[])] : (r.items as Profile[]);
        else posts = more ? dedupe([...posts, ...(r.items as Post[])]) : (r.items as Post[]);
        cursor = r.next_cursor;
      } else if (platform === NET.photofeed) {
        const r = await social.call<{ users: Profile[]; hashtags: { name: string; posts?: number }[] }>("ig_search", { query });
        users = r.users;
        hashtags = r.hashtags;
      } else if (platform === NET.clipfeed && searchKind === "users") {
        const r = await social.call<Page<Profile>>("tt_search", { query, kind: "users", ...(more && cursor ? { cursor } : {}) });
        users = more ? [...users, ...r.items] : r.items;
        cursor = r.next_cursor;
      } else {
        const r = await social.call<Page<Post>>(tool(platform, "search"), { query, ...(more && cursor ? { cursor } : {}) });
        posts = more ? dedupe([...posts, ...r.items]) : r.items;
        cursor = r.next_cursor;
      }
    } catch (e) {
      fail(e);
    } finally {
      loading = false;
    }
  }

  function pickTab(next: Tab) {
    tab = next;
    stack = [];
    void loadTab();
  }

  async function openProfile(handle: string) {
    if (!handle) return;
    stack = [...stack, { kind: "profile", handle }];
    profile = null;
    posts = [];
    cursor = null;
    loading = true;
    error = null;
    try {
      const key = whoKey;
      profile = await social.call<Profile>(profileTool, { [key]: handle });
      const page = await social.call<Page<Post>>(tool(platform, "user_posts"), { [key]: handle });
      posts = page.items;
      cursor = page.next_cursor;
    } catch (e) {
      fail(e);
    } finally {
      loading = false;
    }
  }

  async function moreProfilePosts() {
    if (!profile || !cursor) return;
    loading = true;
    try {
      const page = await social.call<Page<Post>>(tool(platform, "user_posts"), { [whoKey]: profile.handle, cursor });
      posts = dedupe([...posts, ...page.items]);
      cursor = page.next_cursor;
    } catch (e) {
      fail(e);
    } finally {
      loading = false;
    }
  }

  async function openPost(p: Post) {
    if ((platform === NET.codehub && !["issue", "pull"].includes(kindOf(p))) || (platform === NET.forums && kindOf(p) === "comment")) {
      external(p.url);
      return;
    }
    stack = [...stack, { kind: "post", post: p }];
    detail = { post: p };
    replies = [];
    repliesCursor = null;
    replyDraft = "";
    loading = true;
    error = null;
    try {
      const d = await social.call<PostDetail>(postTool, { post: p.url || p.id });
      detail = { ...d, post: d.post ?? p };
      const page = d.replies ?? d.comments;
      replies = page?.items ?? [];
      repliesCursor = page?.next_cursor ?? null;
    } catch (e) {
      fail(e);
    } finally {
      loading = false;
    }
  }

  async function moreReplies() {
    const p = detail?.post;
    if (!p || !repliesCursor) return;
    loading = true;
    try {
      if (platform === NET.shortpost || platform === NET.codehub) {
        const d = await social.call<PostDetail>(postTool, { post: p.id, cursor: repliesCursor });
        const page = d.replies ?? d.comments;
        replies = [...replies, ...(page?.items ?? [])];
        repliesCursor = page?.next_cursor ?? null;
      } else {
        const page = await social.call<Page<Comment>>(tool(platform, "comments"), { post: p.url || p.id, cursor: repliesCursor });
        replies = [...replies, ...page.items];
        repliesCursor = page.next_cursor;
      }
    } catch (e) {
      fail(e);
    } finally {
      loading = false;
    }
  }

  function back() {
    stack = stack.slice(0, -1);
    if (!stack.length) void loadTab();
  }

  function patchPost(id: string, patch: Partial<Post>) {
    const apply = (p: Post) => (p.id === id ? { ...p, ...patch } : p);
    posts = posts.map(apply);
    if (detail?.post) detail = { ...detail, post: apply(detail.post) };
  }

  async function like(p: Post) {
    if (acting[p.id]) return;
    acting = { ...acting, [p.id]: true };
    const undo = !!p.liked;
    patchPost(p.id, { liked: !undo, stats: { ...p.stats, likes: Math.max(0, (p.stats.likes ?? 0) + (undo ? -1 : 1)) } });
    try {
      if (platform === NET.forums) await social.call("rd_vote", { post: p.id.startsWith("t1_") ? p.id : p.url || p.id, direction: undo ? "clear" : "up" });
      else if (platform === NET.codehub) await social.call("gh_star", { repo: p.id, undo });
      else await social.call(tool(platform, "like"), { post: p.url || p.id, undo });
    } catch (e) {
      patchPost(p.id, { liked: undo, stats: p.stats });
      showToast("error", $t(`social.errors.${errKey(e)}`, { platform: label }));
    } finally {
      acting = { ...acting, [p.id]: false };
    }
  }

  async function repost(p: Post) {
    if (acting[p.id]) return;
    acting = { ...acting, [p.id]: true };
    const undo = !!p.reposted;
    patchPost(p.id, { reposted: !undo });
    try {
      await social.call("x_repost", { post: p.id, undo });
    } catch (e) {
      patchPost(p.id, { reposted: undo });
      showToast("error", $t(`social.errors.${errKey(e)}`, { platform: label }));
    } finally {
      acting = { ...acting, [p.id]: false };
    }
  }

  async function follow() {
    if (!profile) return;
    const undo = !!profile.followed_by_me;
    sending = true;
    try {
      if (platform === NET.forums) await social.call("rd_subscribe", { subreddit: profile.handle, undo });
      else await social.call(tool(platform, "follow"), { [whoKey]: profile.handle, undo });
      profile = { ...profile, followed_by_me: !undo };
    } catch (e) {
      showToast("error", $t(`social.errors.${errKey(e)}`, { platform: label }));
    } finally {
      sending = false;
    }
  }

  async function publish() {
    const text = draft.trim();
    if (!text) return;
    sending = true;
    try {
      const r = await social.call<{ post: Post | null }>("x_create_post", { text });
      draft = "";
      if (r.post) posts = [r.post, ...posts];
      showToast("success", $t("social.compose.sent"));
    } catch (e) {
      showToast("error", `${$t(`social.errors.${errKey(e)}`, { platform: label })} ${errDetail(e)}`);
    } finally {
      sending = false;
    }
  }

  async function sendReply() {
    const text = replyDraft.trim();
    if (!text) return;
    sending = true;
    try {
      if (detail?.post) {
        const p = detail.post;
        if (platform === NET.shortpost) {
          const r = await social.call<{ post: Post | null }>("x_create_post", { text, reply_to: p.id });
          if (r.post) replies = [r.post, ...replies];
        } else {
          await social.call(tool(platform, "comment"), { post: p.url || p.id, text });
          replies = [{ id: `local-${Date.now()}`, author: { id: me?.id ?? "", handle: me?.handle ?? "", name: me?.name ?? "", avatar: me?.avatar, verified: false }, text, created_at: Math.floor(Date.now() / 1000) }, ...replies];
        }
      }
      replyDraft = "";
    } catch (e) {
      showToast("error", `${$t(`social.errors.${errKey(e)}`, { platform: label })} ${errDetail(e)}`);
    } finally {
      sending = false;
    }
  }

  async function download(p: Post) {
    const outputDir = getSettings()?.download?.default_output_dir ?? "";
    if (!outputDir) {
      showToast("error", $t("social.post.no_folder"));
      return;
    }
    try {
      await invoke("download_from_url", { url: p.url, outputDir, downloadMode: null, quality: null, formatId: null, referer: null, cookieSlug: null, engine: null });
      showToast("success", $t("social.post.queued"));
    } catch (e) {
      showToast("error", String(e));
    }
  }

  function external(url: string) {
    if (url) void shellOpen(url).catch(() => {});
  }

  function isPost(x: Post | Comment): x is Post {
    return "media" in x;
  }

  onMount(async () => {
    await refreshStatus();
    if (platform === NET.codehub) ghAuth = await social.codehubState().catch(() => null);
    if (loggedIn) await connect();
  });

  onDestroy(stopPoll);
</script>

<svelte:head><title>{label} · OmniGet</title></svelte:head>

<section class="so">
  <header class="so-head">
    <NavIcon icon={SLUG[platform]} size={40} />
    <div class="so-title">
      <h1>{label}</h1>
      <p>{$t(`social.${SLUG[platform]}.subtitle`)}</p>
    </div>
    {#if me}
      <button class="me-chip" onclick={() => openProfile(me?.handle ?? "")} title={$t("social.tabs.me")}>
        {#if me.avatar}<img src={me.avatar} alt="" width="24" height="24" referrerpolicy="no-referrer" />{/if}
        <span>{at(me.handle)}</span>
      </button>
      {#if platform === NET.codehub}
        <button class="btn btn-ghost btn-sm" onclick={() => void openCodehubSetup()}>{$t("social.codehub.setup_open")}</button>
      {/if}
    {/if}
  </header>

  {#if checking && loggedIn === null}
    <div class="boot"><span class="spinner"></span><span>{$t("social.boot")}</span></div>
  {:else if platform === NET.codehub && (!loggedIn || !me)}
    <div class="signin">
      <NavIcon icon="codehub" size={56} />
      <h2>{$t("social.codehub.setup_title")}</h2>
      <p>{$t("social.codehub.setup_hint")}</p>
      <form class="gh-form" onsubmit={(e) => { e.preventDefault(); void saveCodehub({ token: ghToken }); }}>
        <label for="gh-token">{$t("social.codehub.token_label")}</label>
        <div class="gh-row">
          <input id="gh-token" type="password" autocomplete="off" spellcheck="false" bind:value={ghToken} placeholder={$t("social.codehub.token_placeholder")} />
          <button class="btn btn-primary btn-sm" disabled={ghBusy || !ghToken.trim()}>{$t("social.codehub.token_save")}</button>
        </div>
        <span class="gh-note">
          {$t(ghAuth?.has_token ? "social.codehub.token_saved" : "social.codehub.token_note")}
          <button type="button" class="gh-link" onclick={() => external("https://github.com/settings/personal-access-tokens/new")}>{$t("social.codehub.token_create")}</button>
        </span>
      </form>
      <form class="gh-form" onsubmit={(e) => { e.preventDefault(); void saveCodehub({ username: ghUser }); }}>
        <label for="gh-user">{$t("social.codehub.username_label")}</label>
        <div class="gh-row">
          <input id="gh-user" bind:value={ghUser} maxlength="39" autocomplete="off" autocapitalize="off" spellcheck="false" placeholder="octocat" />
          <button class="btn btn-secondary btn-sm" disabled={ghBusy || !ghUser.trim()}>{$t("social.codehub.username_save")}</button>
        </div>
        <span class="gh-note">{$t("social.codehub.username_note")}</span>
      </form>
      {#if error}<p class="err" role="alert">{error}</p>{/if}
      <div class="btn-row">
        {#if loggedIn}
          <button class="btn btn-primary" disabled={checking || ghBusy} onclick={() => void connect()}>
            {#if checking || ghBusy}<span class="spinner"></span>{/if}{$t("social.signin.continue")}
          </button>
        {/if}
        {#if ghAuth?.has_token}
          <button class="btn btn-ghost" disabled={ghBusy} onclick={() => void saveCodehub({ token: "" })}>{$t("social.codehub.token_remove")}</button>
        {/if}
      </div>
    </div>
  {:else if !loggedIn || !me}
    <div class="signin">
      <NavIcon icon={SLUG[platform]} size={56} />
      <h2>{$t("social.signin.title", { platform: label })}</h2>
      <p>{$t("social.signin.hint", { platform: label })}</p>
      <ul>
        <li>{$t("social.signin.point_browser")}</li>
        <li>{$t("social.signin.point_agents")}</li>
        <li>{$t("social.signin.point_risk", { platform: label })}</li>
      </ul>
      {#if error}<p class="err">{error}</p>{/if}
      <div class="btn-row">
        {#if loggedIn}
          <button class="btn btn-primary" disabled={checking} onclick={() => void connect()}>
            {#if checking}<span class="spinner"></span>{/if}{$t("social.signin.continue")}
          </button>
        {:else}
          <button class="btn btn-primary" onclick={() => void signIn()}>{$t("social.signin.button", { platform: label })}</button>
          <button class="btn btn-ghost" disabled={checking} onclick={() => void connect()}>{$t("social.signin.already")}</button>
        {/if}
      </div>
      {#if waitingLogin}<p class="wait"><span class="spinner"></span> {$t("social.signin.waiting", { platform: label })}</p>{/if}
    </div>
  {:else}
    <div class="client">
      <nav class="rail" aria-label={label}>
        {#each tabs as k (k)}
          <button class="rail-btn" class:active={tab === k && view.kind === "tab"} onclick={() => pickTab(k)}>
            <SocialIcon name={TAB_ICON[k]} size={18} />
            <span>{$t(`social.tabs.${k === "saved" && platform === NET.shortpost ? "bookmarks" : k}`)}</span>
          </button>
        {/each}
      </nav>

      <div class="main">
        {#if view.kind !== "tab"}
          <div class="bar">
            <button class="btn btn-ghost btn-sm" onclick={back}><SocialIcon name="back" /> {$t("social.common.back")}</button>
          </div>
        {/if}

        {#if error}
          <div class="notice notice-warning" role="alert">
            <div class="notice-body"><div class="notice-text">{error}</div></div>
            <button class="btn btn-ghost btn-sm" onclick={() => void loadTab()}>{$t("social.common.retry")}</button>
          </div>
        {/if}

        {#if view.kind === "tab" && tab === "audience" && me}
          <AudienceTab {platform} {me} {openProfile} />
        {:else if view.kind === "tab" && tab === "analytics" && me}
          <AnalyticsTab {platform} {me} {openProfile} />
        {:else if view.kind === "tab" && tab === "monitor" && me}
          <MonitorTab {platform} {me} {openProfile} />
        {:else if view.kind === "tab" && tab === "schedule" && me}
          <ScheduleTab {platform} {me} {openProfile} />
        {:else if view.kind === "tab" && tab === "assistant"}
          <div class="assistant-bar">
            <select class="assistant-pick" value={assistantConversation ?? ""} onchange={(e) => void openAssistantChat(e.currentTarget.value)} aria-label={$t("social.assistant.history")}>
              <option value="">{$t("social.assistant.new_chat")}</option>
              {#each assistantChats as c (c.id)}
                <option value={c.id}>{c.title || c.id}</option>
              {/each}
            </select>
            {#if assistantConversation}
              <button class="btn btn-ghost btn-sm" onclick={() => external(`https://x.com/i/grok?conversation=${assistantConversation}`)} title={$t("social.assistant.open_original")}><SocialIcon name="external" /></button>
            {/if}
          </div>
          <div class="list assistant-list">
            {#each assistantMessages as m (m.id)}
              <div class="assistant-msg" class:mine={m.role === "user"}>
                <p>{m.text}</p>
                {#if m.sources.length}
                  <ol class="assistant-sources">
                    {#each m.sources as url (url)}
                      <li><button class="assistant-link" onclick={() => external(url)}>{url.replace(/^https?:\/\/(www\.)?/, "")}</button></li>
                    {/each}
                  </ol>
                {/if}
              </div>
            {:else}
              {#if !assistantBusy}<div class="empty">{$t("social.assistant.empty")}</div>{/if}
            {/each}
            {#if assistantBusy}<div class="boot"><span class="spinner"></span><span>{$t("social.assistant.thinking")}</span></div>{/if}
          </div>
          <form class="reply" onsubmit={(e) => { e.preventDefault(); void askAssistant(); }}>
            <input bind:value={assistantDraft} maxlength="20000" placeholder={$t("social.assistant.placeholder")} />
            <button class="btn btn-primary btn-sm" disabled={assistantBusy || !assistantDraft.trim()} aria-label={$t("social.assistant.send")}><SocialIcon name="send" /></button>
          </form>
        {:else if view.kind === "tab"}
          {#if tab === "feed" && platform === NET.shortpost}
            <div class="compose">
              <textarea bind:value={draft} rows="2" maxlength="4000" placeholder={$t("social.compose.placeholder")}></textarea>
              <div class="compose-row">
                <div class="segmented" role="tablist">
                  <button class="segmented-btn" role="tab" aria-selected={feedMode === "for_you"} onclick={() => { feedMode = "for_you"; void loadTab(); }}>{$t("social.shortpost.for_you")}</button>
                  <button class="segmented-btn" role="tab" aria-selected={feedMode === "following"} onclick={() => { feedMode = "following"; void loadTab(); }}>{$t("social.shortpost.following")}</button>
                </div>
                <span class="grow"></span>
                <span class="count">{draft.length ? `${draft.length}/280` : ""}</span>
                <button class="btn btn-primary btn-sm" disabled={sending || !draft.trim()} onclick={() => void publish()}>{$t("social.compose.publish")}</button>
              </div>
            </div>
          {/if}

          {#if tab === "feed" && FEEDS[platform]}
            <div class="segmented net-feed" role="tablist">
              {#each FEEDS[platform] ?? [] as f (f)}
                <button class="segmented-btn" role="tab" aria-selected={feedPick === f} onclick={() => { netFeed = f; void loadTab(); }}>{$t(`social.${SLUG[platform]}.feed_${f}`)}</button>
              {/each}
            </div>
          {/if}

          {#if tab === "search"}
            <form class="search" onsubmit={(e) => { e.preventDefault(); void loadTab(); }}>
              <input type="search" bind:value={query} placeholder={$t(newer ? `social.${SLUG[platform]}.search_placeholder` : `social.search.placeholder_${SLUG[platform]}`)} />
              {#if KINDS[platform]}
                <div class="segmented" role="tablist">
                  {#each KINDS[platform] ?? [] as k (k.id)}
                    <button type="button" class="segmented-btn" role="tab" aria-selected={kindPick === k.id} onclick={() => (netKind = k.id)}>{$t(`social.${SLUG[platform]}.kind_${k.id}`)}</button>
                  {/each}
                </div>
              {/if}
              {#if platform === NET.clipfeed}
                <div class="segmented" role="tablist">
                  <button type="button" class="segmented-btn" role="tab" aria-selected={searchKind === "posts"} onclick={() => (searchKind = "posts")}>{$t("social.search.videos")}</button>
                  <button type="button" class="segmented-btn" role="tab" aria-selected={searchKind === "users"} onclick={() => (searchKind = "users")}>{$t("social.search.users")}</button>
                </div>
              {/if}
              <button class="btn btn-primary btn-sm" disabled={!query.trim() || loading}>{$t("social.search.go")}</button>
            </form>
            {#if hashtags.length}
              <div class="tags">
                {#each hashtags as h (h.name)}
                  <button class="tag" onclick={() => external(`https://www.instagram.com/explore/tags/${encodeURIComponent(h.name)}/`)}>#{h.name}{#if h.posts} · {compact(h.posts, $locale)}{/if}</button>
                {/each}
              </div>
            {/if}
            {#each users as u (u.id || u.handle)}
              <button class="user-row" onclick={() => openProfile(u.handle)}>
                {#if u.avatar}<img src={u.avatar} alt="" width="40" height="40" referrerpolicy="no-referrer" loading="lazy" />{:else}<span class="avatar-empty">{u.handle.slice(0, 1).toUpperCase()}</span>{/if}
                <span class="who"><strong>{u.name || u.handle}</strong><span>{at(u.handle)}{#if u.followers != null} · {$t("social.profile.followers", { count: compact(u.followers, $locale) })}{/if}</span></span>
              </button>
            {/each}
          {/if}

            <div class="list">
              {#each posts as p (p.id)}
                <PostCard post={p} {platform} busy={!!acting[p.id]} onOpen={openPost} onAuthor={openProfile} onLike={like} onRepost={repost} onDownload={download} onExternal={external} />
              {:else}
                {#if !loading && !error && (tab !== "search" || query.trim()) && !users.length && !hashtags.length}
                  <div class="empty">{$t("social.common.empty")}</div>
                {/if}
              {/each}
            </div>
            {#if loading}
              <div class="boot"><span class="spinner"></span></div>
            {:else if cursor && (posts.length || users.length)}
              <button class="btn btn-ghost more" onclick={() => void loadTab(true)}>{$t("social.common.more")}</button>
            {/if}
        {:else if view.kind === "profile"}
          {#if profile}
            <div class="profile">
              {#if profile.avatar}<img class="pf-avatar" src={profile.avatar} alt="" width="72" height="72" referrerpolicy="no-referrer" />{/if}
              <div class="pf-body">
                <h2>{profile.name || profile.handle}{#if profile.verified}<span class="verified">✓</span>{/if}</h2>
                <span class="pf-handle">{at(profile.handle)}{#if profile.private} · {$t("social.profile.private")}{/if}{#if profile.follows_me} · {$t("social.profile.follows_you")}{/if}</span>
                {#if profile.bio}<p>{profile.bio}</p>{/if}
                <div class="pf-stats">
                  {#if profile.followers != null}<span>{$t("social.profile.followers", { count: compact(profile.followers, $locale) })}</span>{/if}
                  {#if profile.following != null}<span>{$t("social.profile.following", { count: compact(profile.following, $locale) })}</span>{/if}
                  {#if profile.posts != null}<span>{$t(platform === NET.codehub ? "social.codehub.repos" : "social.profile.posts", { count: compact(profile.posts, $locale) })}</span>{/if}
                  {#if profile.likes != null && platform === NET.forums}<span>{$t("social.forums.karma", { count: compact(profile.likes, $locale) })}</span>{/if}
                  {#if profile.likes != null && platform === NET.clipfeed}<span>{$t("social.profile.likes", { count: compact(profile.likes, $locale) })}</span>{/if}
                </div>
              </div>
              <div class="pf-actions">
                {#if canFollow(profile)}
                  <button class="btn btn-sm {profile.followed_by_me ? 'btn-secondary' : 'btn-primary'}" disabled={sending} onclick={() => void follow()}>
                    {$t(platform === NET.forums ? (profile.followed_by_me ? "social.forums.leave" : "social.forums.join") : profile.followed_by_me ? "social.profile.unfollow" : "social.profile.follow")}
                  </button>
                {/if}
                <button class="btn btn-ghost btn-sm" onclick={() => external(profile?.url ?? "")}><SocialIcon name="external" /></button>
              </div>
            </div>
          {/if}
          <div class="list">
            {#each posts as p (p.id)}
              <PostCard post={p} {platform} busy={!!acting[p.id]} onOpen={openPost} onAuthor={openProfile} onLike={like} onRepost={repost} onDownload={download} onExternal={external} />
            {/each}
          </div>
          {#if loading}
            <div class="boot"><span class="spinner"></span></div>
          {:else if cursor}
            <button class="btn btn-ghost more" onclick={() => void moreProfilePosts()}>{$t("social.common.more")}</button>
          {/if}
        {:else if view.kind === "post" && detail?.post}
          <PostCard post={detail.post} {platform} busy={!!acting[detail.post.id]} onAuthor={openProfile} onLike={like} onRepost={repost} onDownload={download} onExternal={external} />
          {#if canReply}
            <form class="reply" onsubmit={(e) => { e.preventDefault(); void sendReply(); }}>
              <input bind:value={replyDraft} maxlength={platform === NET.clipfeed ? 150 : platform === NET.shortpost ? 4000 : 2200} placeholder={$t(platform === NET.shortpost ? "social.compose.reply" : "social.compose.comment")} />
              <button class="btn btn-primary btn-sm" disabled={sending || !replyDraft.trim()}><SocialIcon name="send" /></button>
            </form>
          {/if}
          {#if platform === NET.shortpost}
            <button class="btn btn-ghost btn-sm assistant-ask" onclick={() => detail?.post && askAssistantAbout(detail.post)}><SocialIcon name="spark" /> {$t("social.assistant.ask_about_post")}</button>
          {/if}
          <h3 class="section">{$t(platform === NET.shortpost ? "social.post.replies" : "social.post.comments")}</h3>
          <div class="list">
            {#each replies as r (r.id)}
              {#if isPost(r)}
                <PostCard post={r} {platform} busy={!!acting[r.id]} onOpen={openPost} onAuthor={openProfile} onLike={like} onRepost={repost} onExternal={external} />
              {:else}
                <div class="comment" style:margin-left={`${Math.min(Number(r.extra?.depth ?? 0), 6) * 14}px`}>
                  <button class="c-author" onclick={() => openProfile(r.author.handle)}>
                    {#if r.author.avatar}<img src={r.author.avatar} alt="" width="28" height="28" referrerpolicy="no-referrer" loading="lazy" />{/if}
                    <strong>{at(r.author.handle)}</strong>
                  </button>
                  <p>{r.text}</p>
                  <span class="c-meta">{ago(r.created_at, $locale)}{#if r.likes} · ♥ {compact(r.likes, $locale)}{/if}{#if r.replies} · {$t("social.post.reply_count", { count: r.replies })}{/if}</span>
                </div>
              {/if}
            {:else}
              {#if !loading}<div class="empty">{$t("social.post.no_replies")}</div>{/if}
            {/each}
          </div>
          {#if loading}
            <div class="boot"><span class="spinner"></span></div>
          {:else if repliesCursor}
            <button class="btn btn-ghost more" onclick={() => void moreReplies()}>{$t("social.common.more")}</button>
          {/if}
        {/if}
      </div>
    </div>
  {/if}
</section>

<style>
  .so {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    width: 100%;
    height: 100%;
    max-width: 1040px;
    margin-inline: auto;
    padding: var(--space-4) var(--space-5) var(--space-6);
    min-height: 0;
    overflow: hidden;
  }

  .so-head {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }

  .so-title {
    flex: 1;
    min-width: 0;
  }

  .so-title h1 {
    margin: 0;
    font-family: var(--font-display);
    font-size: var(--text-2xl);
    font-weight: 700;
    letter-spacing: var(--track-tight);
  }

  .so-title p {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .me-chip {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 4px 10px 4px 4px;
    border: 0;
    border-radius: 999px;
    background: var(--fill-1);
    color: var(--text);
    font-size: var(--text-sm);
    cursor: pointer;
  }

  .me-chip img {
    border-radius: 50%;
  }

  .boot {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    padding: var(--space-6) 0;
    color: var(--text-muted);
  }

  .signin {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-3);
    max-width: 520px;
    margin: var(--space-6) auto;
    padding: var(--space-6);
    border-radius: var(--radius-xl, 16px);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    text-align: center;
  }

  .signin h2 {
    margin: 0;
    font-size: var(--text-xl);
  }

  .signin p,
  .signin ul {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--text-sm);
    line-height: 1.5;
  }

  .signin ul {
    text-align: left;
    padding-left: 1.2em;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .btn-row {
    display: flex;
    gap: var(--space-2);
    flex-wrap: wrap;
    justify-content: center;
  }

  .gh-form {
    display: flex;
    flex-direction: column;
    gap: 6px;
    width: 100%;
    text-align: left;
  }

  .gh-form label {
    font-size: var(--text-sm);
    font-weight: 600;
  }

  .gh-row {
    display: flex;
    gap: var(--space-2);
  }

  .gh-row input {
    flex: 1;
    min-width: 0;
  }

  .gh-note {
    font-size: var(--text-xs);
    color: var(--text-dim);
    line-height: 1.5;
  }

  .gh-link {
    padding: 0;
    border: 0;
    background: none;
    color: var(--accent);
    font: inherit;
    cursor: pointer;
    text-decoration: underline;
  }

  .net-feed {
    align-self: flex-start;
  }

  .wait {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .err {
    color: var(--danger) !important;
  }

  .client {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: 180px minmax(0, 1fr);
    gap: var(--space-4);
  }

  .rail {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .rail-btn {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 12px;
    border: 0;
    border-radius: var(--radius-md);
    background: none;
    color: var(--text-dim);
    font-size: var(--text-sm);
    font-weight: 550;
    cursor: pointer;
    text-align: left;
  }

  .rail-btn:hover {
    background: var(--fill-1);
    color: var(--text);
  }

  .rail-btn.active {
    background: var(--fill-2);
    color: var(--text);
  }

  .main {
    min-height: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding-right: 4px;
    padding-bottom: var(--space-4);
  }

  .bar {
    display: flex;
  }

  .list {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .compose,
  .search,
  .reply {
    display: flex;
    gap: var(--space-2);
    padding: var(--space-3);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .compose {
    flex-direction: column;
  }

  .compose textarea {
    resize: vertical;
    min-height: 52px;
    border: 0;
    background: none;
    color: var(--text);
    font: inherit;
    font-size: var(--text-base);
    outline: none;
  }

  .compose-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .count {
    font-size: var(--text-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }

  .grow {
    flex: 1;
  }

  .search input,
  .reply input {
    flex: 1;
    min-width: 0;
  }

  .tags {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }

  .tag {
    padding: 4px 10px;
    border: 0;
    border-radius: 999px;
    background: var(--fill-1);
    color: var(--text);
    font-size: var(--text-xs);
    cursor: pointer;
  }

  .user-row {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-2) var(--space-3);
    border: 0;
    border-radius: var(--radius-md);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    color: var(--text);
    text-align: left;
    cursor: pointer;
  }

  .user-row img,
  .avatar-empty {
    width: 40px;
    height: 40px;
    border-radius: 50%;
    object-fit: cover;
    flex-shrink: 0;
    background: var(--fill-2);
    display: grid;
    place-items: center;
  }


  .who {
    display: flex;
    flex-direction: column;
    min-width: 0;
    font-size: var(--text-sm);
  }

  .who span {
    color: var(--text-dim);
    font-size: var(--text-xs);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .profile {
    display: flex;
    gap: var(--space-4);
    padding: var(--space-4);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .pf-avatar {
    border-radius: 50%;
    object-fit: cover;
    flex-shrink: 0;
  }

  .pf-body {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .pf-body h2 {
    margin: 0;
    font-size: var(--text-xl);
  }

  .pf-body p {
    margin: 4px 0 0;
    white-space: pre-wrap;
    font-size: var(--text-sm);
    overflow-wrap: anywhere;
  }

  .pf-handle,
  .pf-stats {
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .pf-stats {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
    margin-top: 4px;
  }

  .pf-actions {
    display: flex;
    align-items: flex-start;
    gap: 6px;
  }

  .verified {
    margin-left: 6px;
    color: var(--accent);
    font-size: var(--text-sm);
  }

  .assistant-bar {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .assistant-pick {
    flex: 1;
    min-width: 0;
  }

  .assistant-list {
    flex: 1;
  }

  .assistant-msg {
    align-self: flex-start;
    max-width: 88%;
    padding: var(--space-3);
    border-radius: var(--radius-lg, 12px);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .assistant-msg.mine {
    align-self: flex-end;
    background: var(--fill-1);
    box-shadow: none;
  }

  .assistant-msg p {
    margin: 0;
    font-size: var(--text-sm);
    line-height: 1.55;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .assistant-sources {
    margin: var(--space-2) 0 0;
    padding-inline-start: var(--space-5);
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .assistant-link {
    padding: 0;
    border: 0;
    background: none;
    color: var(--accent);
    font: inherit;
    text-align: start;
    cursor: pointer;
    overflow-wrap: anywhere;
  }

  .assistant-ask {
    align-self: flex-start;
  }

  .section {
    margin: var(--space-2) 0 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
    font-weight: 600;
  }

  .comment {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .c-author {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 0;
    border: 0;
    background: none;
    color: var(--text);
    font-size: var(--text-sm);
    cursor: pointer;
  }

  .c-author img {
    border-radius: 50%;
  }

  .comment p {
    margin: 0;
    font-size: var(--text-sm);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .c-meta {
    font-size: var(--text-xs);
    color: var(--text-dim);
  }





  .empty {
    padding: var(--space-6) 0;
    text-align: center;
    color: var(--text-muted);
    font-size: var(--text-sm);
  }

  .more {
    align-self: center;
  }

  @media (max-width: 760px) {
    .client {
      grid-template-columns: 52px minmax(0, 1fr);
    }

    .rail-btn span {
      display: none;
    }
  }
</style>
