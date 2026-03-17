<script lang="ts">
  import { goto } from "$app/navigation";
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { COURSE_PLATFORMS, type CoursePlatform } from "$lib/course-platforms";
  import { t } from "$lib/i18n";

  let searchQuery = $state("");

  let authStatus: Record<string, { checked: boolean; email: string | null; error: boolean }> = $state({});

  let filteredPlatforms = $derived(
    searchQuery.trim() === ""
      ? COURSE_PLATFORMS
      : COURSE_PLATFORMS.filter((p) =>
          p.name.toLowerCase().includes(searchQuery.trim().toLowerCase())
        )
  );

  onMount(() => {
    for (const platform of COURSE_PLATFORMS) {
      if (platform.enabled && platform.authCheckCommand) {
        authStatus[platform.id] = { checked: false, email: null, error: false };
        invoke<string>(platform.authCheckCommand)
          .then((email) => {
            authStatus[platform.id] = { checked: true, email, error: false };
          })
          .catch(() => {
            authStatus[platform.id] = { checked: true, email: null, error: true };
          });
      }
    }
  });

  function handleCardClick(platform: CoursePlatform) {
    if (!platform.enabled) return;
    goto(platform.route);
  }

  function handleKeyDown(e: KeyboardEvent, platform: CoursePlatform) {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      handleCardClick(platform);
    }
  }
</script>

<div class="courses-page">
  <h1>{$t("courses.title")}</h1>

  <input
    class="search-input"
    type="text"
    placeholder={$t("courses.search_placeholder")}
    bind:value={searchQuery}
  />

  <div class="platform-grid">
    {#each filteredPlatforms as platform (platform.id)}
      <div
        class="platform-card"
        class:disabled={!platform.enabled}
        role="button"
        tabindex={platform.enabled ? 0 : -1}
        onclick={() => handleCardClick(platform)}
        onkeydown={(e) => handleKeyDown(e, platform)}
      >
        <div class="card-icon" style="--platform-color: {platform.color}">
          <svg viewBox="0 0 24 24" width="28" height="28" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
            {#if platform.icon === "hotmart"}
              <path d="M7 4v16" /><path d="M17 4v16" /><path d="M7 12h10" />
            {:else if platform.icon === "udemy"}
              <path d="M7 4l5-2 5 2" /><path d="M7 10v6a5 5 0 0 0 10 0v-6" />
            {:else if platform.icon === "kiwify"}
              <path d="M8 4v16" /><path d="M8 12l7-8" /><path d="M8 12l7 8" />
            {:else if platform.icon === "gumroad"}
              <circle cx="12" cy="12" r="9" /><path d="M12 8a4 4 0 1 0 0 8h0v-4" />
            {:else if platform.icon === "teachable"}
              <path d="M6 6h12" /><path d="M12 6v14" />
            {:else if platform.icon === "kajabi"}
              <path d="M8 4v16" /><path d="M8 12l6-8" /><path d="M8 12c2 2 4 5 7 7" />
            {:else if platform.icon === "skool"}
              <path d="M17 7a5 3 0 0 0-10 0c0 2 2 3 5 5s5 1 5 3a5 3 0 0 1-10 0" />
            {:else if platform.icon === "pluralsight"}
              <path d="M8 5v14l11-7z" />
            {:else if platform.icon === "greatcourses"}
              <path d="M4 5l4 14 4-10 4 10 4-14" />
            {:else if platform.icon === "masterclass"}
              <path d="M4 20V6l4 8 4-8 4 8 4-8v14" />
            {:else if platform.icon === "thinkific"}
              <path d="M5 5h14" /><path d="M12 5v15" />
            {:else if platform.icon === "curseduca"}
              <path d="M18 8a8 8 0 1 0-1 9" />
            {:else if platform.icon === "cademi"}
              <path d="M17 7a7 7 0 1 0 0 10" />
            {:else if platform.icon === "cakto"}
              <path d="M12 4v16" /><path d="M12 8h-3v4h3" /><path d="M12 11h3v4h-3" />
            {:else if platform.icon === "kirvano"}
              <path d="M8 4v16" /><path d="M16 4l-8 8 8 8" />
            {:else if platform.icon === "memberkit"}
              <path d="M12 3a4 4 0 1 0 0 8" /><path d="M12 11v9" /><path d="M10 16h4" />
            {:else if platform.icon === "rocketseat"}
              <path d="M12 2l-4 6h8z" /><path d="M12 8v12" /><path d="M9 18l3 4 3-4" />
            {:else if platform.icon === "grancursos"}
              <path d="M18 9a8 8 0 1 0-6 11" /><path d="M12 14h6" />
            {:else if platform.icon === "fluencyacademy"}
              <path d="M4 12a8 8 0 1 1 5 7l-3 3" /><path d="M10 10h4" /><path d="M10 14h2" />
            {:else if platform.icon === "datascienceacademy"}
              <path d="M4 12h4l2-6 4 12 2-6h4" />
            {:else if platform.icon === "medcel"}
              <path d="M12 4v16" /><path d="M4 12h16" />
            {:else if platform.icon === "medcof"}
              <path d="M12 3v18" /><path d="M8 7c0-2 8-2 8 0" /><path d="M8 11c0-2 8-2 8 0" />
            {:else if platform.icon === "medway"}
              <path d="M4 20l4-14 4 8 4-8 4 14" />
            {:else if platform.icon === "afyainternato"}
              <path d="M4 20l8-16 8 16" /><path d="M8 13h8" />
            {:else if platform.icon === "alpaclass"}
              <path d="M4 14l8-4 8 4" /><path d="M4 14v6l8 4 8-4v-6" />
            {:else if platform.icon === "areademembros"}
              <path d="M4 4h16v16H4z" /><path d="M12 9a2 2 0 1 0 0 4" /><path d="M9 15h6" />
            {:else if platform.icon === "astronmembers"}
              <path d="M12 2l3 7h7l-5.5 4.5 2 7L12 16l-6.5 4.5 2-7L2 9h7z" />
            {:else if platform.icon === "eduzznutror"}
              <path d="M17 5H8v14h9" /><path d="M8 12h7" />
            {:else if platform.icon === "entregadigital"}
              <path d="M12 3v12" /><path d="M8 11l4 4 4-4" /><path d="M5 17v2h14v-2" />
            {:else if platform.icon === "greennclub"}
              <path d="M12 20V10" /><path d="M12 10c-4-4-8 0-8 4" /><path d="M12 10c4-4 8 0 8 4" />
            {:else if platform.icon === "themembers"}
              <path d="M4 4h16v12H4z" /><path d="M10 8v4l3-2z" />
            {:else if platform.icon === "voompplay"}
              <path d="M5 5l7 7-7 7" /><path d="M13 5l7 7-7 7" />
            {:else if platform.icon === "estrategia"}
              <path d="M17 5H8v14h9" /><path d="M8 12h6" /><path d="M8 5h9" />
            {:else}
              <path d="M4 19.5A2.5 2.5 0 016.5 17H20" />
              <path d="M6.5 2H20v20H6.5A2.5 2.5 0 014 19.5v-15A2.5 2.5 0 016.5 2z" />
            {/if}
          </svg>
        </div>
        <span class="card-name">{platform.name}</span>
        <span class="card-status">
          {#if !platform.enabled}
            {$t("courses.coming_soon")}
          {:else if authStatus[platform.id]?.checked && authStatus[platform.id]?.error}
            <span class="status-dot error"></span>
            {$t("courses.connection_failed")}
          {:else if authStatus[platform.id]?.checked && authStatus[platform.id]?.email}
            <span class="status-dot connected"></span>
            <span class="status-email">{authStatus[platform.id].email}</span>
          {:else if authStatus[platform.id]?.checked}
            <span class="status-dot disconnected"></span>
            {$t("courses.not_connected")}
          {/if}
        </span>
      </div>
    {/each}
  </div>
</div>

<style>
  .courses-page {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: calc(var(--padding) * 2);
    width: 100%;
  }

  h1 {
    font-size: 20px;
    font-weight: 500;
    margin-block: 0;
    width: 100%;
    max-width: 900px;
  }

  .search-input {
    width: 100%;
    max-width: 900px;
    padding: 10px var(--padding);
    font-size: 14px;
    color: var(--secondary);
    background: var(--input-bg);
    border: 1px solid var(--input-border);
    border-radius: var(--border-radius);
    outline: none;
    box-sizing: border-box;
  }

  .search-input:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  .search-input::placeholder {
    color: var(--gray);
  }

  .platform-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(160px, 1fr));
    gap: var(--padding);
    width: 100%;
    max-width: 900px;
    justify-items: center;
  }

  .platform-card {
    display: flex;
    flex-direction: column;
    align-items: center;
    width: 100%;
    gap: calc(var(--padding) * 0.75);
    padding: calc(var(--padding) * 2) var(--padding);
    background: var(--button-elevated);
    border-radius: var(--border-radius);
    cursor: pointer;
    transition: transform 0.15s, background 0.15s;
  }

  @media (hover: hover) {
    .platform-card:not(.disabled):hover {
      background: var(--sidebar-highlight);
      transform: translateY(-2px);
    }
  }

  .platform-card:not(.disabled):active {
    transform: translateY(0);
  }

  .platform-card:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  .platform-card.disabled {
    opacity: 0.4;
    cursor: default;
  }

  .card-icon {
    width: 52px;
    height: 52px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: calc(var(--border-radius) - 2px);
    background: color-mix(in srgb, var(--platform-color) 15%, transparent);
    color: var(--platform-color);
  }

  .card-name {
    font-size: 14.5px;
    font-weight: 500;
    color: var(--secondary);
  }

  .card-status {
    display: flex;
    align-items: center;
    gap: 5px;
    font-size: 11.5px;
    font-weight: 500;
    color: var(--gray);
    min-height: 16px;
  }

  .status-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  .status-dot.connected {
    background: var(--green);
  }

  .status-dot.disconnected {
    background: var(--gray);
  }

  .status-dot.error {
    background: var(--red);
  }

  .status-email {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 140px;
  }

  @media (max-width: 535px) {
    .platform-grid {
      grid-template-columns: repeat(2, 1fr);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .platform-card {
      transition: none;
    }
  }
</style>
