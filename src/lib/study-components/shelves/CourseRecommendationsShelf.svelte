<script lang="ts">
  import {
    studyLibraryRecommendations,
    type RecommendedCourseItem,
  } from "$lib/study-bridge";
import { t } from "$lib/i18n";
  import Shelf from "./Shelf.svelte";
  import CourseCard from "./CourseCard.svelte";

  type Props = {
    courseId: number;
    limit?: number;
  };

  let { courseId, limit = 6 }: Props = $props();
  let items = $state<RecommendedCourseItem[]>([]);
  let loading = $state(true);

  async function load() {
    loading = true;
    try {
      items = await studyLibraryRecommendations({ courseId, limit });
    } catch (e) {
      console.error("recommendations failed", e);
      items = [];
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    void courseId;
    void load();
  });
</script>

{#if loading}
  <Shelf title={$t("study.course.you_may_like")} eyebrow={$t("study.course.suggestions")} isLoading={true} />
{:else if items.length > 0}
  <Shelf title={$t("study.course.you_may_like")} eyebrow={$t("study.course.suggestions")}>
    {#each items as it (it.id)}
      <CourseCard
        courseId={it.id}
        title={it.title}
        thumbnail={it.thumbnail_path}
        eyebrow={it.platform ?? null}
        tags={it.tags}
      />
    {/each}
  </Shelf>
{/if}
