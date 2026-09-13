<script lang="ts">
  import { convertFileSrc } from "@tauri-apps/api/core";
import { t } from "$lib/i18n";
  import { pluginInvoke } from "$lib/plugin-invoke";
  import NotificationBadge from "./NotificationBadge.svelte";

  type Props = {
    courseId: number;
    title: string;
    thumbnail?: string | null;
    progressPct?: number;
    notificationCount?: number;
    watched?: boolean;
    eyebrow?: string | null;
    href?: string;
    tags?: string[];
  };

  let {
    courseId,
    title,
    thumbnail,
    progressPct,
    notificationCount = 0,
    watched = false,
    eyebrow,
    href,
    tags = [],
  }: Props = $props();

  type SpriteInfo = {
    sprite_url: string;
    cols: number;
    rows: number;
    total_frames: number;
  };
  let sprite = $state<SpriteInfo | null>(null);
  let spriteLoading = false;
  let spriteFailed = false;
  let scrubFrame = $state(0);
  let scrubbing = $state(false);

  async function ensureSprite() {
    if (sprite || spriteLoading || spriteFailed) return;
    spriteLoading = true;
    try {
      const res = await pluginInvoke<{
        sprite_path: string;
        cols: number;
        rows: number;
        total_frames: number;
      }>("study", "study:courses:generate_sprite", { courseId });
      let url = res.sprite_path;
      try {
        url = convertFileSrc(res.sprite_path);
      } catch {
        // keep raw path
      }
      sprite = {
        sprite_url: url,
        cols: res.cols,
        rows: res.rows,
        total_frames: res.total_frames,
      };
    } catch (e) {
      spriteFailed = true;
      if (import.meta.env.DEV) {
        console.warn(`[CourseCard] sprite gen failed for ${courseId}:`, e);
      }
    } finally {
      spriteLoading = false;
    }
  }

  function onPointerEnter() {
    void ensureSprite();
  }
  function onPointerMove(e: PointerEvent) {
    if (!sprite) return;
    const target = e.currentTarget as HTMLElement | null;
    if (!target) return;
    const rect = target.getBoundingClientRect();
    if (rect.width <= 0) return;
    const x = Math.min(rect.width, Math.max(0, e.clientX - rect.left));
    const ratio = x / rect.width;
    scrubFrame = Math.min(
      sprite.total_frames - 1,
      Math.floor(ratio * sprite.total_frames),
    );
    scrubbing = true;
  }
  function onPointerLeave() {
    scrubbing = false;
  }

  const scrubStyle = $derived.by(() => {
    if (!sprite || !scrubbing) return "";
    const col = scrubFrame % sprite.cols;
    const row = Math.floor(scrubFrame / sprite.cols);
    return `background-image: url('${sprite.sprite_url}'); background-position: ${-col * 100}% ${-row * 100}%; background-size: ${sprite.cols * 100}% ${sprite.rows * 100}%;`;
  });

  const visibleTags = $derived(tags.slice(0, 2));
  const extraTags = $derived(Math.max(0, tags.length - 2));

  const link = $derived(href ?? `/study/course/${courseId}`);
  const thumbSrc = $derived.by(() => {
    if (!thumbnail) return null;
    try {
      return convertFileSrc(thumbnail);
    } catch (e) {
      if (import.meta.env.DEV) {
        console.warn(
          `[CourseCard] convertFileSrc failed for course ${courseId}: ${thumbnail}`,
          e,
        );
      }
      return thumbnail;
    }
  });
  const showProgress = $derived(
    typeof progressPct === "number" && progressPct > 0 && progressPct < 1,
  );
  const progressLabel = $derived.by(() => {
    if (typeof progressPct !== "number") return "";
    return `${Math.round(progressPct * 100)}%`;
  });
</script>

<a
  class="card"
  href={link}
  aria-label="Abrir {title}"
  onpointerenter={onPointerEnter}
  onpointermove={onPointerMove}
  onpointerleave={onPointerLeave}
>
  <div class="thumb" style={scrubStyle}>
    {#if thumbSrc && !scrubbing}
      <img src={thumbSrc} alt="" loading="lazy" />
    {:else if !thumbSrc && !scrubbing}
      <div class="thumb-fallback" aria-hidden="true">
        <svg viewBox="0 0 24 24" width="32" height="32" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">
          <rect x="3" y="6" width="18" height="12" rx="2" />
          <polygon points="10,9 15,12 10,15" fill="currentColor" stroke="none" />
        </svg>
      </div>
    {/if}
    <NotificationBadge count={notificationCount} />
    {#if visibleTags.length > 0 && notificationCount === 0}
      <div class="tag-chips" aria-hidden="true">
        {#each visibleTags as tag (tag)}
          <span class="tag-chip">#{tag}</span>
        {/each}
        {#if extraTags > 0}
          <span class="tag-chip more">+{extraTags}</span>
        {/if}
      </div>
    {/if}
    {#if watched}
      <span class="watched" aria-label={$t("study.course.completed_aria")}>✓</span>
    {/if}
    {#if showProgress}
      <div class="progress" role="progressbar" aria-valuemin="0" aria-valuemax="100" aria-valuenow={Math.round((progressPct ?? 0) * 100)}>
        <div class="progress-fill" style:width={progressLabel}></div>
      </div>
    {/if}
  </div>
  <div class="meta">
    {#if eyebrow}
      <span class="eyebrow">{eyebrow}</span>
    {/if}
    <span class="title">{title}</span>
  </div>
</a>

<style>
  .card {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: 8px;
    width: 240px;
    flex: 0 0 240px;
    text-decoration: none;
    color: inherit;
    border-radius: 12px;
    transition: transform var(--tg-duration-fast, 150ms) ease;
  }

  .card:hover,
  .card:focus-visible {
    transform: scale(1.03);
    outline: none;
  }

  .card:focus-visible .thumb {
    box-shadow: 0 0 0 3px color-mix(in oklab, var(--accent) 60%, transparent);
  }

  .thumb {
    position: relative;
    aspect-ratio: 16 / 9;
    border-radius: 10px;
    overflow: hidden;
    background: var(--card-bg, color-mix(in oklab, var(--content-bg) 80%, var(--accent) 4%));
    box-shadow: 0 2px 6px color-mix(in oklab, black 8%, transparent);
  }

  .thumb img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }

  .thumb-fallback {
    display: grid;
    place-items: center;
    width: 100%;
    height: 100%;
    color: color-mix(in oklab, currentColor 30%, transparent);
  }

  .tag-chips {
    position: absolute;
    top: 8px;
    right: 8px;
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    max-width: 70%;
    justify-content: flex-end;
  }
  .tag-chip {
    font-size: 10px;
    font-weight: 500;
    padding: 2px 7px;
    border-radius: 999px;
    background: color-mix(in oklab, black 60%, transparent);
    color: white;
    backdrop-filter: blur(6px);
    border: 1px solid color-mix(in oklab, white 14%, transparent);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 80px;
  }
  .tag-chip.more {
    background: color-mix(in oklab, var(--accent) 60%, black);
    border-color: color-mix(in oklab, var(--accent) 50%, transparent);
  }
  .watched {
    position: absolute;
    top: 8px;
    left: 8px;
    width: 22px;
    height: 22px;
    border-radius: 50%;
    background: var(--success);
    color: white;
    font-size: 13px;
    font-weight: 700;
    display: grid;
    place-items: center;
    box-shadow: 0 2px 6px color-mix(in oklab, var(--success) 40%, transparent);
  }

  .progress {
    position: absolute;
    inset: auto 0 0 0;
    height: 4px;
    background: color-mix(in oklab, black 40%, transparent);
  }

  .progress-fill {
    height: 100%;
    background: var(--accent);
    transition: width 200ms ease;
  }

  .meta {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 0 4px;
  }

  .eyebrow {
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: color-mix(in oklab, currentColor 60%, transparent);
    font-weight: 600;
  }

  .title {
    font-size: 14px;
    font-weight: 500;
    line-height: 1.3;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }

  @media (prefers-reduced-motion: reduce) {
    .card,
    .progress-fill {
      transition: none;
    }
    .card:hover,
    .card:focus-visible {
      transform: none;
    }
  }
</style>
