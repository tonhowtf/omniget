<script lang="ts">
  import { onMount } from "svelte";
  import { beforeNavigate, goto } from "$app/navigation";
  import type { ExcalidrawImperativeAPI, ExcalidrawInitialDataState } from "@excalidraw/excalidraw/types";
  import { t, locale } from "$lib/i18n";
  import { getAppearancePresentation } from "$lib/stores/settings-store.svelte";
  import * as api from "$lib/board/api";
  import { Autosaver, hasUnsaved, initialSaveState, type SaveState } from "$lib/board/autosave";
  import { loadKit, parseScene, serialize, signature, thumbnail, toPng, toSvg, isEmpty, type Kit, type Scene } from "$lib/board/excalidraw";
  import { upsert } from "$lib/board/library";
  import { excalidrawLang } from "$lib/board/locale";
  import { checkName, sanitizeName } from "$lib/board/names";
  import type { BoardEntry, BoardFile, ExportKind, ExportOptions } from "$lib/board/types";
  import NavIcon from "#components/shell/NavIcon.svelte";
  import LibraryPanel from "#components/board/LibraryPanel.svelte";
  import ExcalidrawHost from "#components/board/ExcalidrawHost.svelte";
  import SaveBadge from "#components/board/SaveBadge.svelte";
  import ExportMenu from "#components/board/ExportMenu.svelte";
  import BoardIcon from "#components/board/BoardIcon.svelte";

  type NoticeAction = { label: string; run: () => void; danger?: boolean };
  type Notice = { tone: "info" | "success" | "error"; text: string; actions: NoticeAction[] };
  type Editor =
    | { phase: "none" }
    | { phase: "loading"; path: string }
    | { phase: "error"; path: string; message: string }
    | { phase: "ready"; file: BoardFile; initial: ExcalidrawInitialDataState };

  const AUTOSAVE_MS = 1500;

  let boards = $state<BoardEntry[]>([]);
  let folder = $state("");
  let isDefault = $state(true);
  let libLoading = $state(true);
  let libError = $state<string | null>(null);
  let thumbRevision = $state<Record<string, number>>({});

  let kit = $state<Kit | null>(null);
  let kitError = $state<string | null>(null);
  let editor = $state<Editor>({ phase: "none" });
  let excalidraw: ExcalidrawImperativeAPI | null = null;
  let lastScene: { file: BoardFile; scene: Scene } | null = null;
  const retired = new WeakSet<BoardFile>();
  let lastSignature = "";
  let saveState = $state<SaveState>(initialSaveState());
  let saver: Autosaver | null = null;
  let pathLock: Promise<unknown> = Promise.resolve();

  let titleDraft = $state("");
  let titleError = $state<string | null>(null);
  let titleBusy = $state(false);

  let notice = $state<Notice | null>(null);
  let exportOpen = $state(false);
  let exportBusy = $state<ExportKind | "copy" | null>(null);
  let exportOpts = $state<ExportOptions>({ scale: 2, background: true, dark: false });
  let sceneEmpty = $state(true);
  let leaving = false;

  let theme = $derived(getAppearancePresentation().resolvedMode);
  let lang = $derived(excalidrawLang($locale));
  let current = $derived(editor.phase === "ready" ? editor.file : null);
  let unsaved = $derived(hasUnsaved(saveState));
  let saveFailing = $derived(saveState.status.kind === "error");

  function describe(error: unknown): string {
    const { code, detail } = api.errorCode(error);
    const base = $t(api.errorKey(code)) as string;
    return code === "unknown" || code === "BOARD_IO" ? `${base}${detail ? ` (${detail})` : ""}` : base;
  }

  function say(tone: Notice["tone"], text: string, ...actions: (NoticeAction | undefined)[]) {
    notice = { tone, text, actions: actions.filter((a): a is NoticeAction => a !== undefined) };
  }

  async function loadLibrary() {
    libLoading = true;
    libError = null;
    try {
      const lib = await api.library();
      boards = lib.boards;
      folder = lib.folder;
      isDefault = lib.is_default;
    } catch (e) {
      if (api.errorCode(e).code === "BOARD_NO_FOLDER") {
        boards = [];
        isDefault = false;
      }
      libError = $t("board.library_error", { reason: describe(e) }) as string;
    } finally {
      libLoading = false;
    }
  }

  function currentScene(): Scene | null {
    if (!excalidraw) return lastScene && lastScene.file === current ? lastScene.scene : null;
    return { elements: excalidraw.getSceneElements(), appState: excalidraw.getAppState(), files: excalidraw.getFiles() };
  }

  async function writeCurrent() {
    const file = current;
    const scene = currentScene();
    const k = kit;
    if (!file || !scene || !k) throw new Error("BOARD_NOT_FOUND");
    const contents = serialize(k, scene);
    await pathLock;
    if (retired.has(file)) throw new Error("BOARD_NOT_FOUND");
    let thumb: string | null = null;
    if (!file.external) {
      thumb = await thumbnail(k, scene)
        .then(api.blobToBase64)
        .catch(() => null);
    }
    const info = await api.save(file.path, contents, thumb);
    if (current?.path === file.path) current.modified_ms = info.modified_ms;
    if (!file.external) {
      const prev = boards.find((b) => b.path === file.path);
      if (prev) boards = upsert(boards, { ...prev, modified_ms: info.modified_ms, size: info.size, has_thumb: prev.has_thumb || thumb !== null });
      thumbRevision[file.path] = (thumbRevision[file.path] ?? 0) + 1;
    }
  }

  function makeSaver() {
    saver?.dispose();
    saver = new Autosaver({
      delayMs: AUTOSAVE_MS,
      save: () => writeCurrent(),
      onState: (s) => (saveState = s),
      describe,
    });
    saveState = saver.current;
  }

  async function settleCurrent(next?: () => void): Promise<boolean> {
    if (!saver || !hasUnsaved(saver.current)) return true;
    const ok = await saver.flush();
    if (!ok && saver) {
      const status = saver.current.status;
      say(
        "error",
        $t("board.leave_blocked", { reason: status.kind === "error" ? status.reason : "" }) as string,
        {
          label: $t("board.retry_save") as string,
          run: () =>
            void settleCurrent(next).then((saved) => {
              if (saved) next?.();
            }),
        },
        { label: $t("board.save_copy") as string, run: () => void exportAs("excalidraw") },
        next
          ? {
              label: $t("board.discard_and_continue") as string,
              danger: true,
              run: () => {
                closeEditor();
                next();
              },
            }
          : undefined,
      );
    }
    return ok;
  }

  async function holdPath<T>(work: () => Promise<T>): Promise<T> {
    await saver?.idle();
    let release: () => void = () => {};
    const lock = new Promise<void>((r) => (release = r));
    const previous = pathLock;
    pathLock = previous.then(() => lock);
    try {
      await previous;
      return await work();
    } finally {
      release();
    }
  }

  async function openPath(path: string, preloaded?: BoardFile) {
    if (current?.path === path && !preloaded) return;
    if (!(await settleCurrent(() => void openPath(path, preloaded)))) return;
    exportOpen = false;
    editor = { phase: "loading", path };
    if (!kit) {
      try {
        kit = await loadKit();
        kitError = null;
      } catch (e) {
        kitError = api.errorText(e);
        editor = { phase: "error", path, message: $t("board.kit_error", { reason: kitError }) as string };
        return;
      }
    }
    try {
      const file = preloaded ?? (await api.read(path));
      const initial = parseScene(file.contents);
      lastSignature = "";
      lastScene = null;
      makeSaver();
      editor = { phase: "ready", file, initial };
      titleDraft = file.name;
      titleError = null;
      sceneEmpty = (initial.elements?.length ?? 0) === 0;
    } catch (e) {
      const message = e instanceof SyntaxError ? ($t("board.err.not_scene") as string) : describe(e);
      editor = { phase: "error", path, message };
    }
  }

  function closeEditor() {
    saver?.dispose();
    saver = null;
    excalidraw = null;
    lastScene = null;
    saveState = initialSaveState();
    editor = { phase: "none" };
  }

  function onSceneChange() {
    if (!kit || !saver) return;
    const scene = currentScene();
    if (!scene) return;
    const sig = signature(kit, scene);
    if (!lastSignature) {
      lastSignature = sig;
      return;
    }
    if (sig === lastSignature) return;
    lastSignature = sig;
    sceneEmpty = isEmpty(kit, scene);
    saver.change();
  }

  function onReady(instance: ExcalidrawImperativeAPI | null) {
    if (!instance && excalidraw && current) {
      const scene = currentScene();
      lastScene = scene ? { file: current, scene } : null;
    }
    if (instance) lastScene = null;
    excalidraw = instance;
  }

  async function createBoard() {
    try {
      const entry = await api.create($t("board.untitled") as string);
      boards = upsert(boards, entry);
      await openPath(entry.path);
    } catch (e) {
      say("error", $t("board.create_error", { reason: describe(e) }) as string);
    }
  }

  async function openFromDisk() {
    try {
      const file = await api.openFile($t("board.open_file_title") as string);
      if (!file) return;
      if (!file.external) boards = upsert(boards, boards.find((b) => b.path === file.path) ?? { name: file.name, path: file.path, modified_ms: file.modified_ms, size: file.contents.length, has_thumb: false });
      await openPath(file.path, file);
    } catch (e) {
      say("error", $t("board.open_error", { reason: describe(e) }) as string);
    }
  }

  async function importBoards() {
    try {
      const added = await api.importFiles($t("board.import_title") as string);
      if (added.length === 0) return;
      for (const entry of added) boards = upsert(boards, entry);
      say("success", (added.length === 1 ? $t("board.imported_one", { name: added[0].name }) : $t("board.imported_many", { count: added.length })) as string);
      if (added.length === 1) await openPath(added[0].path);
    } catch (e) {
      say("error", $t("board.import_error", { reason: describe(e) }) as string);
    }
  }

  async function renameBoard(path: string, name: string): Promise<string | null> {
    const isOpen = current?.path === path;
    if (isOpen && !(await settleCurrent())) return $t("board.rename_blocked") as string;
    try {
      return await holdPath(async () => {
        const entry = await api.rename(path, name);
        if (current && current.path === path) {
          current.path = entry.path;
          current.name = entry.name;
          titleDraft = entry.name;
        }
        if (!current?.external || current.path !== entry.path) boards = upsert(boards, entry, path);
        return null;
      });
    } catch (e) {
      return describe(e);
    }
  }

  async function commitTitle() {
    if (!current || titleBusy) return;
    const others = current.external ? [] : boards.filter((b) => b.path !== current?.path).map((b) => b.name);
    const check = checkName(titleDraft, others, current.name);
    if (!check.ok) {
      titleError = $t(`board.name_${check.error}`) as string;
      return;
    }
    titleError = null;
    if (check.name === current.name) {
      titleDraft = current.name;
      return;
    }
    titleBusy = true;
    const failure = await renameBoard(current.path, check.name);
    titleBusy = false;
    if (failure) titleError = failure;
  }

  function titleKey(event: KeyboardEvent) {
    if (event.key === "Enter") {
      event.preventDefault();
      (event.currentTarget as HTMLInputElement).blur();
    } else if (event.key === "Escape" && current) {
      titleDraft = current.name;
      titleError = null;
      (event.currentTarget as HTMLInputElement).blur();
    }
  }

  async function duplicateBoard(entry: BoardEntry) {
    if (current?.path === entry.path && !(await settleCurrent())) return;
    try {
      const copy = await api.duplicate(entry.path, $t("board.copy_suffix") as string);
      boards = upsert(boards, copy);
      say("success", $t("board.duplicated", { name: copy.name }) as string, { label: $t("board.open") as string, run: () => void openPath(copy.path) });
    } catch (e) {
      say("error", $t("board.duplicate_error", { reason: describe(e) }) as string);
    }
  }

  async function deleteBoard(entry: BoardEntry) {
    if (current?.path === entry.path && !(await settleCurrent(() => void deleteBoard(entry)))) return;
    try {
      const result = await holdPath(async () => {
        const removed = await api.remove(entry.path);
        if (current && current.path === entry.path) {
          retired.add(current);
          closeEditor();
        }
        return removed;
      });
      boards = boards.filter((b) => b.path !== entry.path);
      if (result.method === "folder" && result.restore_path) {
        const parked = result.restore_path;
        say("info", $t("board.deleted_folder", { name: entry.name }) as string, {
          label: $t("board.undo") as string,
          run: () => void restoreBoard(parked),
        });
      } else {
        say("info", $t("board.deleted_os", { name: entry.name }) as string);
      }
    } catch (e) {
      say("error", $t("board.delete_error", { reason: describe(e) }) as string);
    }
  }

  async function restoreBoard(parked: string) {
    try {
      const entry = await api.restore(parked);
      boards = upsert(boards, entry);
      say("success", $t("board.restored", { name: entry.name }) as string);
    } catch (e) {
      say("error", $t("board.restore_error", { reason: describe(e) }) as string);
    }
  }

  async function changeFolder() {
    if (!(await settleCurrent())) return;
    try {
      const lib = await api.chooseFolder($t("board.choose_folder_title") as string);
      if (!lib) return;
      if (current && !current.external) closeEditor();
      boards = lib.boards;
      folder = lib.folder;
      isDefault = lib.is_default;
      say("success", $t("board.folder_changed", { count: lib.boards.length }) as string);
    } catch (e) {
      say("error", $t("board.folder_error", { reason: describe(e) }) as string);
    }
  }

  async function resetFolder() {
    if (!(await settleCurrent())) return;
    try {
      const lib = await api.resetFolder();
      if (current && !current.external) closeEditor();
      boards = lib.boards;
      folder = lib.folder;
      isDefault = lib.is_default;
    } catch (e) {
      say("error", $t("board.folder_error", { reason: describe(e) }) as string);
    }
  }

  function exportName(): string {
    return sanitizeName(current?.name ?? "", $t("board.untitled") as string);
  }

  async function exportAs(kind: ExportKind) {
    const scene = currentScene();
    if (!kit || !scene || exportBusy) return;
    exportBusy = kind;
    try {
      let data: string;
      if (kind === "png") data = await api.blobToBase64(await toPng(kit, scene, exportOpts));
      else if (kind === "svg") data = await toSvg(kit, scene, exportOpts);
      else data = serialize(kit, scene);
      const saved = await api.exportFile(exportName(), kind, data, $t("board.export_dialog_title") as string);
      if (!saved) return;
      exportOpen = false;
      say("success", $t("board.exported", { path: saved }) as string, { label: $t("board.show_in_folder") as string, run: () => void api.reveal(saved).catch(() => {}) });
    } catch (e) {
      say("error", $t("board.export_error", { reason: describe(e) }) as string);
    } finally {
      exportBusy = null;
    }
  }

  async function copyPng() {
    const scene = currentScene();
    if (!kit || !scene || exportBusy) return;
    exportBusy = "copy";
    try {
      const blob = await toPng(kit, scene, exportOpts);
      try {
        await api.copyPng(await api.blobToBase64(blob));
      } catch (nativeError) {
        if (typeof ClipboardItem === "undefined" || !navigator.clipboard?.write) throw nativeError;
        await navigator.clipboard.write([new ClipboardItem({ "image/png": blob })]);
      }
      exportOpen = false;
      say("success", $t("board.copied_png") as string);
    } catch (e) {
      say("error", $t("board.copy_error", { reason: describe(e) }) as string);
    } finally {
      exportBusy = null;
    }
  }

  function onKeydown(event: KeyboardEvent) {
    if ((event.metaKey || event.ctrlKey) && !event.shiftKey && !event.altKey && event.key.toLowerCase() === "s") {
      if (editor.phase !== "ready") return;
      event.preventDefault();
      event.stopPropagation();
      void saver?.flush();
    }
  }

  function retrySave() {
    void saver?.retryNow();
  }

  beforeNavigate((nav) => {
    if (leaving || !saver || !hasUnsaved(saver.current)) return;
    if (nav.type === "leave") {
      void saver.flush();
      return;
    }
    nav.cancel();
    const target = nav.to?.url;
    const go = (): void => {
      if (saver && hasUnsaved(saver.current)) {
        void settleCurrent(go).then((ok) => {
          if (ok) go();
        });
        return;
      }
      leaving = true;
      if (target) void goto(target);
    };
    go();
  });

  onMount(() => {
    void loadLibrary();
    void loadKit()
      .then((k) => (kit = k))
      .catch((e) => (kitError = api.errorText(e)));
    window.addEventListener("keydown", onKeydown, true);
    const flushOnHide = () => {
      if (document.visibilityState === "hidden") void saver?.flush();
    };
    document.addEventListener("visibilitychange", flushOnHide);
    return () => {
      window.removeEventListener("keydown", onKeydown, true);
      document.removeEventListener("visibilitychange", flushOnHide);
      void saver?.flush().finally(() => saver?.dispose());
    };
  });
</script>

<section class="bd">
  <header class="bd-head">
    <NavIcon icon="board" size={56} />
    <div class="bd-head-copy">
      <h1>{$t("board.title")}</h1>
      <p class="bd-subtitle">{$t("board.subtitle")}</p>
    </div>
  </header>

  {#if notice}
    <div class="bd-notice bd-notice-{notice.tone}" role={notice.tone === "error" ? "alert" : "status"}>
      <BoardIcon name={notice.tone === "error" ? "warning" : "check"} size={16} />
      <p>{notice.text}</p>
      {#each notice.actions as action, i (action.label)}
        <button
          type="button"
          class="btn btn-sm {i === 0 ? 'btn-secondary' : 'btn-ghost'}"
          class:bd-discard={action.danger}
          onclick={() => {
            const run = action.run;
            notice = null;
            run();
          }}>{action.label}</button
        >
      {/each}
      <button type="button" class="btn btn-ghost btn-icon btn-sm" aria-label={$t("board.dismiss")} onclick={() => (notice = null)}>
        <BoardIcon name="close" size={13} />
      </button>
    </div>
  {/if}

  <div class="bd-body">
    <LibraryPanel
      {boards}
      {folder}
      {isDefault}
      activePath={current && !current.external ? current.path : null}
      loading={libLoading}
      loadError={libError}
      {thumbRevision}
      onopen={(entry) => void openPath(entry.path)}
      oncreate={createBoard}
      onimport={importBoards}
      onopenfile={openFromDisk}
      onrename={(entry, name) => renameBoard(entry.path, name)}
      onduplicate={duplicateBoard}
      ondelete={deleteBoard}
      onchoosefolder={changeFolder}
      onresetfolder={resetFolder}
      onreload={loadLibrary}
    />

    <div class="bd-main">
      {#if editor.phase === "ready" && kit}
        {@const file = editor.file}
        <div class="bd-bar">
          <div class="bd-title">
            <label class="sr-only" for="board-title">{$t("board.title_label")}</label>
            <input
              id="board-title"
              class="bd-title-input"
              class:invalid={titleError}
              bind:value={titleDraft}
              disabled={titleBusy}
              aria-invalid={titleError ? "true" : undefined}
              aria-describedby={titleError ? "board-title-error" : file.external ? "board-external" : undefined}
              onkeydown={titleKey}
              onblur={commitTitle}
            />
            {#if titleError}
              <p id="board-title-error" class="bd-title-error" role="alert">{titleError}</p>
            {:else if file.external}
              <p id="board-external" class="bd-external" title={file.path}>{$t("board.external_file", { path: file.path })}</p>
            {/if}
          </div>
          <SaveBadge status={saveState.status} onretry={retrySave} />
          <div class="bd-export">
            <button type="button" class="btn btn-secondary" aria-expanded={exportOpen} aria-haspopup="dialog" onclick={() => (exportOpen = !exportOpen)}>
              <BoardIcon name="export" size={14} />
              {$t("board.export")}
            </button>
            {#if exportOpen}
              <ExportMenu bind:opts={exportOpts} busy={exportBusy} empty={sceneEmpty} onexport={exportAs} oncopy={copyPng} onclose={() => (exportOpen = false)} />
            {/if}
          </div>
        </div>
        {#if saveFailing && unsaved}
          <div class="bd-unsaved" role="alert">
            <BoardIcon name="warning" size={14} />
            <p>{$t("board.unsaved_kept")}</p>
            <button type="button" class="btn btn-ghost btn-sm" onclick={() => void exportAs("excalidraw")}>{$t("board.save_copy")}</button>
          </div>
        {/if}
        <div class="bd-canvas">
          {#key editor.initial}
            <ExcalidrawHost {kit} initial={editor.initial} {theme} {lang} label={$t("board.canvas_label", { name: file.name })} onchange={onSceneChange} onready={onReady} />
          {/key}
        </div>
      {:else if editor.phase === "loading" || (editor.phase === "ready" && !kit)}
        <div class="bd-state" aria-busy="true">
          <span class="spinner" aria-hidden="true"></span>
          <p>{$t("board.opening")}</p>
        </div>
      {:else if editor.phase === "error"}
        {@const failedPath = editor.path}
        <div class="bd-state" role="alert">
          <BoardIcon name="warning" size={28} />
          <h2>{$t("board.open_failed_title")}</h2>
          <p>{editor.message}</p>
          <div class="bd-state-actions">
            <button type="button" class="btn btn-secondary" onclick={closeEditor}>{$t("board.back_to_library")}</button>
            <button type="button" class="btn btn-primary" onclick={() => { editor = { phase: "none" }; void openPath(failedPath); }}>{$t("board.try_again")}</button>
          </div>
        </div>
      {:else}
        <div class="bd-state bd-empty">
          <svg class="bd-empty-art" viewBox="0 0 64 48" width="72" height="54" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <rect x="4" y="4" width="56" height="40" rx="6" />
            <path d="M14 32c4-9 8-12 12-6s8 3 12-6" />
            <path d="M44 14h8M48 10v8" />
          </svg>
          {#if boards.length === 0 && !libLoading && !libError}
            <h2>{$t("board.empty_title")}</h2>
            <p>{$t("board.empty_body")}</p>
            <div class="bd-state-actions">
              <button type="button" class="btn btn-secondary" onclick={openFromDisk}>{$t("board.open_file")}</button>
              <button type="button" class="btn btn-primary" onclick={createBoard}>{$t("board.new_board")}</button>
            </div>
          {:else}
            <h2>{$t("board.pick_title")}</h2>
            <p>{$t("board.pick_body")}</p>
          {/if}
          {#if kitError}
            <p class="bd-kit-error" role="alert">{$t("board.kit_error", { reason: kitError })}</p>
          {/if}
        </div>
      {/if}
    </div>
  </div>
</section>

<style>
  .bd {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    flex: 1;
    min-height: 0;
    width: 100%;
    padding: var(--space-4) var(--space-5) var(--space-4);
  }

  .bd-head {
    display: flex;
    align-items: center;
    gap: var(--space-4);
  }

  .bd-head-copy {
    min-width: 0;
  }

  .bd-head h1 {
    margin: 0 0 var(--space-1);
    font-family: var(--font-display);
    font-size: var(--text-2xl);
    font-weight: 700;
    letter-spacing: var(--track-tight);
    color: var(--text);
  }

  .bd-subtitle {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--text-base);
  }

  .bd-notice {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-2) var(--space-2) var(--space-4);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .bd-notice p {
    flex: 1;
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text);
    overflow-wrap: anywhere;
  }

  .bd-notice-success > :global(.bi) {
    color: var(--success);
  }

  .bd-notice-info > :global(.bi) {
    color: var(--blue);
  }

  .bd-notice-error > :global(.bi) {
    color: var(--error);
  }

  .bd-discard {
    color: var(--error);
  }

  .bd-body {
    flex: 1;
    min-height: 420px;
    display: flex;
    gap: var(--space-3);
  }

  .bd-main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    overflow: hidden;
  }

  .bd-bar {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-2) var(--space-3);
    border-bottom: var(--hairline) solid var(--content-border);
  }

  .bd-title {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }

  .bd-title-input {
    width: 100%;
    max-width: 420px;
    height: var(--control-h);
    padding: 0 var(--space-2);
    margin-inline-start: calc(var(--space-2) * -1);
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text);
    font-size: var(--text-md);
    font-weight: 600;
  }

  .bd-title-input:hover {
    box-shadow: inset 0 0 0 var(--hairline) var(--input-border);
  }

  .bd-title-input:focus-visible {
    outline: 2px solid var(--focus-ring);
    background: var(--pane-bg);
  }

  .bd-title-input.invalid {
    box-shadow: inset 0 0 0 1px var(--error);
  }

  .bd-title-error,
  .bd-external {
    margin: 0;
    font-size: var(--text-xs);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .bd-title-error {
    color: var(--error);
  }

  .bd-external {
    color: var(--text-dim);
  }

  .bd-export {
    position: relative;
    flex-shrink: 0;
  }

  .bd-export > .btn {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
  }

  .bd-unsaved {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-1) var(--space-3);
    background: color-mix(in srgb, var(--error) 10%, transparent);
    color: var(--text);
    font-size: var(--text-sm);
  }

  .bd-unsaved > :global(.bi) {
    color: var(--error);
  }

  .bd-unsaved p {
    flex: 1;
    margin: 0;
  }

  .bd-canvas {
    flex: 1;
    min-height: 0;
    position: relative;
  }

  .bd-state {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-3);
    padding: var(--space-7) var(--space-5);
    text-align: center;
    color: var(--text-dim);
  }

  .bd-state h2 {
    margin: 0;
    font-size: var(--text-lg);
    font-weight: 600;
    color: var(--text);
  }

  .bd-state p {
    margin: 0;
    max-width: 420px;
    font-size: var(--text-base);
    overflow-wrap: anywhere;
  }

  .bd-state-actions {
    display: flex;
    gap: var(--space-2);
    margin-top: var(--space-2);
  }

  .bd-empty-art {
    color: var(--text-muted);
  }

  .bd-kit-error {
    color: var(--error);
    font-size: var(--text-sm);
  }

  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    margin: -1px;
    padding: 0;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
    border: 0;
  }

  @media (max-width: 760px) {
    .bd-body {
      flex-direction: column;
    }

    .bd-body > :global(.lp) {
      width: 100%;
      max-height: 280px;
    }
  }
</style>
