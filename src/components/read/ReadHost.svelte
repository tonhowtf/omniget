<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { getDownloads } from "$lib/stores/download-store.svelte";
  import { registerSemantic } from "$lib/native-agent/semantic";
  import { readDownloads, syncReadDownloads } from "$lib/read/downloads.svelte";
  import { reader } from "$lib/read/reader-state.svelte";

  onMount(() => {
    const id = setTimeout(() => void invoke("read_status").catch(() => undefined), 4000);
    return () => clearTimeout(id);
  });

  onMount(() =>
    registerSemantic("read-reader", () => ({
      ready: true,
      open: !!reader.title,
      title: reader.title || null,
      kind: reader.kind || null,
      location: reader.location || null,
      progress: Math.round(reader.progress * 1000) / 10,
      chapter: reader.chapter || null,
      selection: reader.selection || null,
      stage: reader.stage || null,
    })),
  );

  $effect(() => {
    const map = getDownloads();
    if (readDownloads.items.some((x) => x.state === "running")) syncReadDownloads(map);
  });
</script>
