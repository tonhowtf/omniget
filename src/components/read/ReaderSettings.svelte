<script lang="ts">
  import { t } from "$lib/i18n";
  import { FONTS, THEMES } from "$lib/read/foliate";
  import { applyPreset, deletePreset, presets, prefs, resetPrefs, savePreset, savePrefs } from "$lib/read/prefs.svelte";

  let { onchange, fixed = false }: { onchange: () => void; fixed?: boolean } = $props();
  let tab = $state<"text" | "layout" | "theme" | "more">("text");
  let list = $state(presets());
  let presetName = $state("");

  function set<K extends keyof typeof prefs>(k: K, v: (typeof prefs)[K]) {
    prefs[k] = v;
    savePrefs();
    onchange();
  }
</script>

<div class="rs">
  <div class="tabs">
    {#each ["text", "layout", "theme", "more"] as x (x)}
      <button class:on={tab === x} onclick={() => (tab = x as typeof tab)}>{$t(`read.rs.tab_${x}`)}</button>
    {/each}
  </div>

  {#if tab === "text"}
    {#if fixed}<p class="hint">{$t("read.rs.fixed_hint")}</p>{/if}
    <h4>{$t("read.rs.font")}</h4>
    <div class="fonts">
      {#each FONTS as f (f.id)}
        <button class:on={prefs.fontFamily === f.id} style:font-family={f.css || "inherit"} onclick={() => set("fontFamily", f.id)}>{f.label}</button>
      {/each}
    </div>
    <label class="row">{$t("read.rs.size")} <span>{prefs.fontSize}px</span>
      <input type="range" min="10" max="40" value={prefs.fontSize} oninput={(e) => set("fontSize", Number((e.target as HTMLInputElement).value))} />
    </label>
    <label class="row">{$t("read.rs.line_height")} <span>{prefs.lineHeight.toFixed(2)}</span>
      <input type="range" min="1" max="2.4" step="0.05" value={prefs.lineHeight} oninput={(e) => set("lineHeight", Number((e.target as HTMLInputElement).value))} />
    </label>
    <label class="row">{$t("read.rs.weight")} <span>{prefs.fontWeight}</span>
      <input type="range" min="300" max="700" step="100" value={prefs.fontWeight} oninput={(e) => set("fontWeight", Number((e.target as HTMLInputElement).value))} />
    </label>
    <label class="row">{$t("read.rs.paragraph_spacing")} <span>{prefs.paragraphSpacing}em</span>
      <input type="range" min="0" max="2" step="0.1" value={prefs.paragraphSpacing} oninput={(e) => set("paragraphSpacing", Number((e.target as HTMLInputElement).value))} />
    </label>
    <label class="row">{$t("read.rs.indent")} <span>{prefs.textIndent < 0 ? $t("read.rs.publisher") : `${prefs.textIndent}em`}</span>
      <input type="range" min="-1" max="3" step="0.5" value={prefs.textIndent} oninput={(e) => set("textIndent", Number((e.target as HTMLInputElement).value))} />
    </label>
    <label class="row">{$t("read.rs.letter_spacing")} <span>{prefs.letterSpacing}em</span>
      <input type="range" min="-0.05" max="0.2" step="0.01" value={prefs.letterSpacing} oninput={(e) => set("letterSpacing", Number((e.target as HTMLInputElement).value))} />
    </label>
    <label class="row">{$t("read.rs.word_spacing")} <span>{prefs.wordSpacing}em</span>
      <input type="range" min="-0.1" max="0.5" step="0.02" value={prefs.wordSpacing} oninput={(e) => set("wordSpacing", Number((e.target as HTMLInputElement).value))} />
    </label>
    <label class="ck"><input type="checkbox" checked={prefs.justify} onchange={(e) => set("justify", (e.target as HTMLInputElement).checked)} /> {$t("read.rs.justify")}</label>
    <label class="ck"><input type="checkbox" checked={prefs.hyphenate} onchange={(e) => set("hyphenate", (e.target as HTMLInputElement).checked)} /> {$t("read.rs.hyphenate")}</label>
    <label class="ck"><input type="checkbox" checked={prefs.overrideFont} onchange={(e) => set("overrideFont", (e.target as HTMLInputElement).checked)} /> {$t("read.rs.override_font")}</label>
    <label class="ck"><input type="checkbox" checked={prefs.bionic} onchange={(e) => set("bionic", (e.target as HTMLInputElement).checked)} /> {$t("read.rs.bionic")}</label>
  {:else if tab === "layout"}
    <h4>{$t("read.rs.flow")}</h4>
    <div class="seg">
      <button class:on={prefs.flow === "paginated"} onclick={() => set("flow", "paginated")}>{$t("read.rs.paginated")}</button>
      <button class:on={prefs.flow === "scrolled"} onclick={() => set("flow", "scrolled")}>{$t("read.rs.scrolled")}</button>
    </div>
    <h4>{$t("read.rs.columns")}</h4>
    <div class="seg">
      {#each [1, 2, 3] as c (c)}
        <button class:on={prefs.maxColumns === c} onclick={() => set("maxColumns", c)}>{c}</button>
      {/each}
    </div>
    <label class="row">{$t("read.rs.width")} <span>{prefs.maxWidth}px</span>
      <input type="range" min="400" max="1400" step="20" value={prefs.maxWidth} oninput={(e) => set("maxWidth", Number((e.target as HTMLInputElement).value))} />
    </label>
    <label class="row">{$t("read.rs.margin")} <span>{prefs.margin}px</span>
      <input type="range" min="0" max="120" step="4" value={prefs.margin} oninput={(e) => set("margin", Number((e.target as HTMLInputElement).value))} />
    </label>
    <label class="row">{$t("read.rs.gap")} <span>{prefs.gap}%</span>
      <input type="range" min="0" max="15" value={prefs.gap} oninput={(e) => set("gap", Number((e.target as HTMLInputElement).value))} />
    </label>
    <label class="ck"><input type="checkbox" checked={prefs.animated} onchange={(e) => set("animated", (e.target as HTMLInputElement).checked)} /> {$t("read.rs.animated")}</label>
    <label class="row">{$t("read.rs.ruler")} <span>{prefs.rulerLines ? prefs.rulerLines : $t("read.rs.off")}</span>
      <input type="range" min="0" max="5" value={prefs.rulerLines} oninput={(e) => set("rulerLines", Number((e.target as HTMLInputElement).value))} />
    </label>
  {:else if tab === "theme"}
    <div class="themes">
      {#each Object.entries(THEMES) as [id, th] (id)}
        <button class:on={prefs.theme === id} style:background={th.bg} style:color={th.fg} onclick={() => set("theme", id)}>Aa<span>{th.name}</span></button>
      {/each}
    </div>
    <label class="row">{$t("read.reader.brightness")} <span>{prefs.brightness}%</span>
      <input type="range" min="40" max="130" value={prefs.brightness} oninput={(e) => set("brightness", Number((e.target as HTMLInputElement).value))} />
    </label>
    <label class="ck"><input type="checkbox" checked={prefs.overrideColors} onchange={(e) => set("overrideColors", (e.target as HTMLInputElement).checked)} /> {$t("read.rs.override_colors")}</label>
    <label class="ck"><input type="checkbox" checked={prefs.invertImages} onchange={(e) => set("invertImages", (e.target as HTMLInputElement).checked)} /> {$t("read.rs.invert_images")}</label>
  {:else}
    <h4>{$t("read.rs.custom_css")}</h4>
    <textarea class="input css" rows="5" value={prefs.customCss} onchange={(e) => set("customCss", (e.target as HTMLTextAreaElement).value)} placeholder="p {'{'} text-align: left {'}'}"></textarea>
    <h4>{$t("read.rs.presets")}</h4>
    <div class="presets">
      {#each list as p (p.name)}
        <span class="pr">
          <button onclick={() => { applyPreset(p); onchange(); }}>{p.name}</button>
          <button class="x" onclick={() => { deletePreset(p.name); list = presets(); }} aria-label={$t("read.delete") as string}>×</button>
        </span>
      {/each}
    </div>
    <form class="row" onsubmit={(e) => { e.preventDefault(); if (presetName.trim()) { savePreset(presetName.trim()); presetName = ""; list = presets(); } }}>
      <input class="input" bind:value={presetName} placeholder={$t("read.rs.preset_name") as string} />
      <button class="btn" type="submit">{$t("read.save")}</button>
    </form>
    <button class="btn" onclick={() => { resetPrefs(); onchange(); }}>{$t("read.rs.reset")}</button>
  {/if}
</div>

<style>
  .rs {
    display: flex;
    flex-direction: column;
    gap: 8px;
    font-size: 13px;
  }
  .tabs,
  .seg {
    display: flex;
    gap: 4px;
  }
  .tabs button,
  .seg button {
    all: unset;
    flex: 1;
    text-align: center;
    padding: 6px 4px;
    border-radius: 7px;
    background: var(--fill-1);
    cursor: pointer;
    font-size: 12px;
  }
  .tabs button.on,
  .seg button.on {
    background: var(--accent);
    color: #fff;
  }
  h4 {
    margin: 6px 0 0;
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-muted);
  }
  .hint {
    color: var(--text-muted);
    margin: 0;
  }
  .row {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 2px 8px;
    align-items: center;
  }
  .row span {
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }
  .row input[type="range"] {
    grid-column: 1 / -1;
    accent-color: var(--accent);
  }
  form.row {
    grid-template-columns: 1fr auto;
  }
  .ck {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .fonts {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 4px;
  }
  .fonts button {
    all: unset;
    text-align: center;
    padding: 8px 4px;
    border-radius: 7px;
    background: var(--fill-1);
    cursor: pointer;
  }
  .fonts button.on {
    outline: 2px solid var(--accent);
  }
  .themes {
    display: grid;
    grid-template-columns: repeat(5, 1fr);
    gap: 6px;
  }
  .themes button {
    all: unset;
    display: flex;
    flex-direction: column;
    align-items: center;
    padding: 8px 2px;
    border-radius: 8px;
    font-family: Georgia, serif;
    font-size: 16px;
    cursor: pointer;
    box-shadow: inset 0 0 0 1px rgb(128 128 128 / 0.4);
  }
  .themes button span {
    font-family: system-ui;
    font-size: 9px;
  }
  .themes button.on {
    outline: 2px solid var(--accent);
  }
  .css {
    font-family: var(--font-mono);
    font-size: 12px;
  }
  .presets {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .pr {
    display: inline-flex;
    border-radius: 7px;
    background: var(--fill-1);
  }
  .pr button {
    all: unset;
    padding: 4px 8px;
    cursor: pointer;
  }
  .pr .x {
    padding: 4px 6px;
    color: var(--text-muted);
  }
</style>
