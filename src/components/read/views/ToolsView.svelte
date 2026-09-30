<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { open } from "@tauri-apps/plugin-dialog";
  import { open as openUrl } from "@tauri-apps/plugin-shell";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { bookById, coverUrl, library } from "$lib/read/library-store.svelte";
  import { fmtBytes, go, tr } from "$lib/read/ui-store.svelte";
  import { isComic, type Book, type Device, type DeviceProfile, type Tool } from "$lib/read/types";
  import { agentActions } from "$lib/native-agent/actions";
  import Cover from "../Cover.svelte";
  import ReadIcon from "../ReadIcon.svelte";

  let { tab = "convert", ids = [] }: { tab?: string; ids?: number[] } = $props();

  let current = $state(untrack(() => tab));
  let lib = $derived(library());
  let picked = $state<number[]>(untrack(() => [...ids]));
  let files = $state<string[]>([]);
  let tools = $state<Tool[]>([]);
  let profiles = $state<DeviceProfile[]>([]);
  let devices = $state<Device[]>([]);
  let to = $state("epub");
  let engine = $state("auto");
  let comic = $state({ device: "", crop: true, split: true, grayscale: true, rtl: false, quality: 85, upscale: false });
  let outputToLibrary = $state(true);
  let addToLibrary = $state(true);
  let extra = $state("");
  let busy = $state(false);
  let mailTo = $state("");
  let deviceMount = $state("");
  let subfolder = $state("");

  onMount(async () => {
    [tools, profiles, devices] = await Promise.all([
      invoke<Tool[]>("read_tools").catch(() => []),
      invoke<DeviceProfile[]>("read_devices").catch(() => []),
      invoke<Device[]>("read_devices_detect").catch(() => []),
    ]);
    deviceMount = devices[0]?.mount ?? "";
  });

  let books = $derived(picked.map((id) => bookById(id)).filter((b): b is Book => !!b));
  let anyComic = $derived(books.some((b) => isComic(b)) || files.some((f) => /\.(cbz|cbr|cb7|zip)$/i.test(f)));
  let has = (id: string) => tools.some((x) => x.id === id && x.path);

  async function addFiles() {
    const f = await open({ multiple: true });
    if (Array.isArray(f)) files = [...files, ...f];
    else if (typeof f === "string") files = [...files, f];
  }

  async function convert() {
    busy = true;
    try {
      const status = lib.status;
      await invoke("read_convert", {
        req: {
          book_ids: picked,
          paths: files,
          to,
          engine,
          comic,
          extra_args: extra.trim() ? extra.trim().split(/\s+/) : [],
          output_dir: outputToLibrary ? status?.download_dir ?? null : null,
          add_to_library: addToLibrary,
        },
      });
      showToast("success", tr("read.tools.started"));
      go({ kind: "downloads" });
    } catch (e) {
      showToast("error", String(e));
    } finally {
      busy = false;
    }
  }

  async function sendDevice() {
    busy = true;
    try {
      const skipped = await invoke<string[]>("read_send_to_device", { bookIds: picked, mount: deviceMount, subfolder: subfolder || null });
      if (skipped.length) showToast("info", tr("read.tools.skipped", { list: skipped.join(", ") }));
      else showToast("success", tr("read.tools.sent", { count: picked.length }));
    } catch (e) {
      showToast("error", String(e));
    } finally {
      busy = false;
    }
  }

  async function sendMail() {
    busy = true;
    try {
      const n = await invoke<number>("read_send_email", { bookIds: picked, to: mailTo || null });
      showToast("success", tr("read.tools.mailed", { count: n }));
    } catch (e) {
      showToast("error", String(e));
    } finally {
      busy = false;
    }
  }

  let dev = $derived(devices.find((d) => d.mount === deviceMount));
</script>

<div class="r-h">
  <h1>{$t("read.nav.tools")}</h1>
  <span class="grow"></span>
  <div class="mac-segmented">
    {#each ["convert", "send", "status"] as x (x)}
      <button class="mac-segmented-btn" data-agent-actions="click" use:agentActions={["click"]} class:active={current === x} onclick={() => (current = x)}>{$t(`read.tools.tab_${x}`)}</button>
    {/each}
  </div>
</div>

{#if current !== "status"}
  <div class="r-panel">
    <div class="r-line">
      <b>{$t("read.tools.items", { count: books.length + files.length })}</b>
      <span style="flex:1"></span>
      <button class="r-chip" onclick={() => go({ kind: "library" })}>{$t("read.tools.pick_library")}</button>
      {#if current === "convert"}<button class="r-chip" onclick={addFiles}>{$t("read.tools.pick_files")}</button>{/if}
    </div>
    <div class="picked">
      {#each books as b (b.id)}
        <span class="pk">
          <span class="c"><Cover src={coverUrl(b.cover)} title={b.title} radius={2} /></span>{b.title} <em>{b.format}</em>
          <button onclick={() => (picked = picked.filter((x) => x !== b.id))} aria-label={$t("read.remove") as string}>×</button>
        </span>
      {/each}
      {#each files as f (f)}
        <span class="pk">{f.split(/[/\\]/).pop()} <button onclick={() => (files = files.filter((x) => x !== f))} aria-label={$t("read.remove") as string}>×</button></span>
      {/each}
      {#if !books.length && !files.length}<p class="r-muted">{$t("read.tools.none_picked")}</p>{/if}
    </div>
  </div>
{/if}

{#if current === "convert"}
  <div class="r-panel">
    <div class="r-fields">
      <label class="r-field">{$t("read.tools.to")}
        <select class="input" bind:value={to}>
          {#each ["epub", "kepub", "azw3", "mobi", "pdf", "cbz", "fb2", "docx", "txt", "html", "md"] as f (f)}<option value={f}>{f.toUpperCase()}</option>{/each}
          <option value="m4b">{$t("read.tools.audiobook")}</option>
        </select>
      </label>
      <label class="r-field">{$t("read.tools.engine")}
        <select class="input" bind:value={engine}>
          <option value="auto">{$t("read.tools.engine_auto")}</option>
          <option value="native">{$t("read.tools.engine_native")}</option>
          <option value="calibre" disabled={!has("ebook-convert")}>calibre</option>
          <option value="kcc" disabled={!has("kcc-c2e")}>KCC</option>
          <option value="pandoc" disabled={!has("pandoc")}>pandoc</option>
          <option value="kepubify" disabled={!has("kepubify")}>kepubify</option>
          <option value="gcc" disabled={!has("go-comic-converter")}>go-comic-converter</option>
          <option value="markitdown" disabled={!has("markitdown") && !has("uv")}>MarkItDown</option>
          <option value="audiobook" disabled={!has("ebook2audiobook")}>ebook2audiobook</option>
        </select>
      </label>
    </div>
    {#if anyComic}
      <h3 class="h3">{$t("read.tools.comic_options")}</h3>
      <div class="r-fields">
        <label class="r-field">{$t("read.tools.device")}
          <select class="input" bind:value={comic.device}>
            <option value="">{$t("read.tools.no_resize")}</option>
            {#each profiles as p (p.id)}<option value={p.id}>{p.name} ({p.width}×{p.height})</option>{/each}
          </select>
        </label>
        <label class="r-field">{$t("read.tools.quality")} {comic.quality}<input type="range" min="50" max="100" bind:value={comic.quality} /></label>
      </div>
      <div class="r-line ops">
        <label><input type="checkbox" bind:checked={comic.rtl} /> {$t("read.tools.manga_rtl")}</label>
        <label><input type="checkbox" bind:checked={comic.crop} /> {$t("read.comic.crop")}</label>
        <label><input type="checkbox" bind:checked={comic.split} /> {$t("read.tools.split")}</label>
        <label><input type="checkbox" bind:checked={comic.grayscale} /> {$t("read.tools.grayscale")}</label>
        <label><input type="checkbox" bind:checked={comic.upscale} /> {$t("read.tools.upscale")}</label>
      </div>
    {/if}
    <div class="r-line ops">
      <label><input type="checkbox" bind:checked={outputToLibrary} /> {$t("read.tools.output_library")}</label>
      <label><input type="checkbox" bind:checked={addToLibrary} /> {$t("read.tools.add_library")}</label>
    </div>
    <label class="r-field">{$t("read.tools.extra_args")}<input class="input" bind:value={extra} placeholder="--output-profile kobo" /></label>
    <div class="dialog-actions">
      <button class="btn btn-primary" onclick={convert} disabled={busy || (!books.length && !files.length)}><ReadIcon name="convert" size={15} /> {$t("read.tools.convert")}</button>
    </div>
    <p class="r-muted">{$t("read.tools.native_hint")}</p>
  </div>
{:else if current === "send"}
  <div class="r-panel">
    <h3 class="h3"><ReadIcon name="tablet" size={15} /> {$t("read.tools.usb")}</h3>
    {#if !devices.length}
      <p class="r-muted">{$t("read.tools.no_device")}</p>
    {:else}
      <div class="r-fields">
        <label class="r-field">{$t("read.tools.device")}
          <select class="input" bind:value={deviceMount}>
            {#each devices as d (d.mount)}<option value={d.mount}>{d.name} · {d.kind}{d.free_bytes != null ? ` · ${fmtBytes(d.free_bytes)} free` : ""}</option>{/each}
          </select>
        </label>
        <label class="r-field">{$t("read.tools.subfolder")}<input class="input" bind:value={subfolder} /></label>
      </div>
      {#if dev}<p class="r-muted">{$t("read.tools.reads", { list: dev.formats.join(", ").toUpperCase() })}</p>{/if}
    {/if}
    <div class="dialog-actions">
      <button class="btn" onclick={async () => (devices = await invoke<Device[]>("read_devices_detect"))}><ReadIcon name="refresh" size={14} /> {$t("read.tools.rescan")}</button>
      <button class="btn btn-primary" onclick={sendDevice} disabled={busy || !deviceMount || !books.length}><ReadIcon name="send" size={14} /> {$t("read.tools.send")}</button>
    </div>
  </div>
  <div class="r-panel">
    <h3 class="h3"><ReadIcon name="mail" size={15} /> {$t("read.tools.email")}</h3>
    <p class="r-muted">{$t("read.tools.email_hint")}</p>
    <div class="r-line">
      <input class="input" style="flex:1" bind:value={mailTo} placeholder="you@kindle.com" />
      <button class="btn" onclick={() => go({ kind: "settings", tab: "devices" })}>{$t("read.tools.email_setup")}</button>
      <button class="btn btn-primary" onclick={sendMail} disabled={busy || !books.length}><ReadIcon name="send" size={14} /> {$t("read.tools.send")}</button>
    </div>
  </div>
{:else}
  <div class="r-panel">
    <p class="r-muted">{$t("read.tools.status_hint")}</p>
    <ul class="r-list">
      {#each tools as tl (tl.id)}
        <li class="r-li">
          <ReadIcon name={tl.path ? "check" : "alert"} size={15} />
          <div class="ti"><b>{tl.name}</b><span class="r-muted">{tl.path ?? $t("read.tools.not_found")}{tl.version ? ` · ${tl.version}` : ""}</span></div>
          <button
            class="btn"
            onclick={async () => {
              const f = await open({ multiple: false });
              if (typeof f !== "string") return;
              try {
                await invoke("read_tool_set_path", { id: tl.id, path: f });
                tools = await invoke<Tool[]>("read_tools");
              } catch (e) {
                showToast("error", String(e));
              }
            }}>{$t("read.tools.set_path")}</button
          >
          {#if !tl.path}<button class="btn" onclick={() => openUrl(tl.url)}>{$t("read.tools.get")}</button>{/if}
        </li>
      {/each}
    </ul>
  </div>
{/if}

<style>
  .picked {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-top: var(--space-2);
  }
  .pk {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 3px 4px 3px 4px;
    border-radius: var(--radius-sm);
    background: var(--fill-1);
    font-size: var(--text-sm);
  }
  .pk .c {
    width: 18px;
  }
  .pk em {
    font-style: normal;
    color: var(--text-muted);
    text-transform: uppercase;
    font-size: var(--text-xs);
  }
  .pk button {
    all: unset;
    cursor: pointer;
    padding: 0 4px;
    color: var(--text-muted);
  }
  .h3 {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: var(--space-3) 0 var(--space-2);
    font-size: var(--text-base);
  }
  .ops {
    margin: var(--space-3) 0;
    font-size: var(--text-sm);
    gap: var(--space-4);
  }
  .ti {
    flex: 1;
    display: flex;
    flex-direction: column;
  }
</style>
