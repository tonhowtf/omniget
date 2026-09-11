<script lang="ts">
  /**
   * Seis tools de CTF num componente: hash, tipo de arquivo, cifras, XOR,
   * codificações e frequência. `mode` decide qual formulário aparece — todas
   * têm a mesma forma (entra texto ou arquivo, sai resultado) e tudo é local.
   */
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { baseName, errText, fmtBytes, pickFile } from "$lib/tools/rt";

  let { mode = "hash" }: { mode?: "hash" | "magic" | "cipher" | "xor" | "encode" | "freq" } = $props();

  type Hashes = { crc32: string; md5: string; sha1: string; sha256: string; sha512: string; bytes: number };
  type HashGuess = { name: string; confidence: string; note: string };
  type TypeGuess = { label: string; extension: string; mime: string; offset: number };
  type Magic = { path: string; bytes: number; guess: TypeGuess | null; embedded: TypeGuess[]; entropy: number; strings: string[]; extension_mismatch: boolean };
  type Caesar = { shift: number; text: string; score: number };
  type XorCand = { key: string; key_hex: string; text: string; score: number };
  type XorRes = { candidates: XorCand[]; key_sizes: [number, number][] };
  type Encoded = { output: string; bytes: number; error: string | null };
  type FreqEntry = { byte: number; display: string; count: number; percent: number };
  type Freq = { total: number; distinct: number; entropy: number; index_of_coincidence: number; top: FreqEntry[]; looks_like: string };

  let text = $state("");
  let file = $state("");
  let fromFile = $state(false);
  let busy = $state(false);

  // hash
  let hashes = $state<Hashes | null>(null);
  let hmacKey = $state("");
  let hmacAlg = $state("sha256");
  let hmacOut = $state("");
  let hashInput = $state("");
  let guesses = $state<HashGuess[]>([]);
  // magic
  let magic = $state<Magic | null>(null);
  let minLen = $state(6);
  // cipher
  let cipher = $state("caesar");
  let shift = $state(3);
  let key = $state("");
  let decrypt = $state(false);
  let cipherOut = $state("");
  let caesar = $state<Caesar[]>([]);
  // xor
  let xorMode = $state("single");
  let xorHexIn = $state(false);
  let xorKey = $state("");
  let xorKeyHex = $state(false);
  let xorRes = $state<XorRes | null>(null);
  // encode
  let format = $state("base64");
  let decode = $state(false);
  let encoded = $state<Encoded | null>(null);
  let detected = $state<string[]>([]);
  // freq
  let freq = $state<Freq | null>(null);

  async function copy(v: string) {
    await navigator.clipboard.writeText(v);
    showToast("success", $t("tools.common.copied") as string);
  }

  async function choose() {
    const f = await pickFile();
    if (f) {
      file = f;
      fromFile = true;
    }
  }

  async function run() {
    if (busy) return;
    const source = fromFile ? file : text;
    if (!source) return;
    busy = true;
    try {
      if (mode === "hash") {
        hashes = await invoke<Hashes>("tool_ctf_hash", { input: source, isFile: fromFile });
      } else if (mode === "magic") {
        magic = await invoke<Magic>("tool_ctf_magic", { path: file, minLen, limit: 200 });
      } else if (mode === "cipher") {
        cipherOut = await invoke<string>("tool_ctf_cipher", {
          opts: { text, cipher, decrypt, shift, key },
        });
      } else if (mode === "xor") {
        xorRes = await invoke<XorRes>("tool_ctf_xor", {
          opts: { input: text, input_hex: xorHexIn, mode: xorMode, key: xorKey, key_hex: xorKeyHex, max_key_size: 16 },
        });
      } else if (mode === "encode") {
        encoded = await invoke<Encoded>("tool_ctf_encode", { opts: { input: text, format, decode } });
        if (encoded.error) showToast("error", encoded.error);
      } else {
        freq = await invoke<Freq>("tool_ctf_freq", { input: source, isFile: fromFile });
      }
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }

  async function identify() {
    if (!hashInput.trim()) return;
    guesses = await invoke<HashGuess[]>("tool_ctf_hash_id", { input: hashInput });
  }

  async function makeHmac() {
    hmacOut = await invoke<string>("tool_ctf_hmac", {
      opts: { data: text, key: hmacKey, algorithm: hmacAlg },
    });
  }

  async function brute() {
    if (!text) return;
    caesar = await invoke<Caesar[]>("tool_ctf_caesar_brute", { text });
  }

  async function detect() {
    detected = await invoke<string[]>("tool_ctf_detect", { input: text });
    if (detected.length) format = detected[0];
  }

  const HASH_ROWS: [string, keyof Hashes][] = [
    ["CRC-32", "crc32"], ["MD5", "md5"], ["SHA-1", "sha1"], ["SHA-256", "sha256"], ["SHA-512", "sha512"],
  ];
</script>

<div class="tool">
  <section>
    <div class="group">
      {#if mode === "magic"}
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.ctf.file")}</div><div class="group-row-sub mono">{file ? baseName(file) : "—"}</div></div>
          <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={choose}>{$t("tools.common.choose")}</button></div>
        </div>
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.ctf.min_len")} {minLen}</div></div>
          <div class="group-row-trailing"><input type="range" min="4" max="20" step="1" bind:value={minLen} /></div>
        </div>
      {:else}
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.ctf.input")}</div><div class="group-row-sub">{$t("tools.ctf.offline")}</div></div>
          {#if mode === "hash" || mode === "freq"}
            <div class="group-row-trailing btn-row">
              <button class="toggle" class:on={fromFile} type="button" role="switch" aria-checked={fromFile} aria-label={$t("tools.ctf.from_file")} onclick={() => (fromFile = !fromFile)}><span class="toggle-knob"></span></button>
              {#if fromFile}<button class="btn btn-secondary btn-sm" type="button" onclick={choose}>{file ? baseName(file) : $t("tools.common.choose")}</button>{/if}
            </div>
          {/if}
        </div>
        {#if !fromFile}
          <div class="group-row block">
            <textarea class="input area mono" rows="5" bind:value={text} placeholder=""></textarea>
          </div>
        {/if}
      {/if}

      {#if mode === "cipher"}
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.ctf.cipher")}</div></div>
          <div class="group-row-trailing btn-row">
            <select class="input" bind:value={cipher}>
              <option value="caesar">César / ROT</option>
              <option value="atbash">Atbash</option>
              <option value="vigenere">Vigenère</option>
              <option value="railfence">Rail Fence</option>
            </select>
            {#if cipher === "caesar" || cipher === "railfence"}
              <input class="input" type="number" min={cipher === "railfence" ? 2 : -25} max="25" bind:value={shift} style:width="5em" />
            {/if}
            {#if cipher === "vigenere"}<input class="input mono" type="text" bind:value={key} placeholder="LEMON" style:width="8em" />{/if}
          </div>
        </div>
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.ctf.decrypt")}</div></div>
          <div class="group-row-trailing"><button class="toggle" class:on={decrypt} type="button" role="switch" aria-checked={decrypt} aria-label={$t("tools.ctf.decrypt")} onclick={() => (decrypt = !decrypt)}><span class="toggle-knob"></span></button></div>
        </div>
      {/if}

      {#if mode === "xor"}
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.ctf.mode")}</div></div>
          <div class="group-row-trailing btn-row">
            <select class="input" bind:value={xorMode}>
              <option value="single">{$t("tools.ctf.single")}</option>
              <option value="repeating">{$t("tools.ctf.repeating")}</option>
              <option value="apply">{$t("tools.ctf.apply")}</option>
            </select>
            <button class="toggle" class:on={xorHexIn} type="button" role="switch" aria-checked={xorHexIn} aria-label={$t("tools.ctf.hex_input")} onclick={() => (xorHexIn = !xorHexIn)}><span class="toggle-knob"></span></button>
            <span class="dim">{$t("tools.ctf.hex_input")}</span>
          </div>
        </div>
        {#if xorMode === "apply"}
          <div class="group-row">
            <div class="group-row-content"><div class="group-row-title">{$t("tools.ctf.key")}</div></div>
            <div class="group-row-trailing btn-row">
              <input class="input mono" type="text" bind:value={xorKey} style:width="10em" />
              <button class="toggle" class:on={xorKeyHex} type="button" role="switch" aria-checked={xorKeyHex} aria-label={$t("tools.ctf.hex_key")} onclick={() => (xorKeyHex = !xorKeyHex)}><span class="toggle-knob"></span></button>
              <span class="dim">hex</span>
            </div>
          </div>
        {/if}
      {/if}

      {#if mode === "encode"}
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.ctf.format")}</div>{#if detected.length}<div class="group-row-sub">{$t("tools.ctf.detected")}: {detected.join(", ")}</div>{/if}</div>
          <div class="group-row-trailing btn-row">
            <select class="input" bind:value={format}>
              <option value="base64">base64</option><option value="base32">base32</option>
              <option value="hex">hex</option><option value="url">URL</option>
              <option value="html">HTML</option><option value="binary">binário</option>
              <option value="decimal">decimal</option><option value="morse">morse</option>
            </select>
            <button class="toggle" class:on={decode} type="button" role="switch" aria-checked={decode} aria-label={$t("tools.ctf.decode")} onclick={() => (decode = !decode)}><span class="toggle-knob"></span></button>
            <span class="dim">{$t("tools.ctf.decode")}</span>
            <button class="btn btn-ghost btn-sm" type="button" onclick={detect}>?</button>
          </div>
        </div>
      {/if}

      <div class="group-row">
        <div class="group-row-content"></div>
        <div class="group-row-trailing btn-row">
          {#if mode === "cipher"}<button class="btn btn-secondary" type="button" disabled={!text} onclick={brute}>{$t("tools.ctf.brute")}</button>{/if}
          <button class="btn btn-primary" type="button" disabled={busy || (mode === "magic" ? !file : !(fromFile ? file : text))} onclick={run}>{busy ? $t("tools.common.working") : $t("tools.ctf.run")}</button>
        </div>
      </div>
    </div>
  </section>

  {#if mode === "hash"}
    {#if hashes}
      <section><div class="group">
        {#each HASH_ROWS as [label, k] (k)}
          <div class="group-row">
            <div class="group-row-content"><div class="group-row-title">{label}</div><div class="group-row-sub mono">{hashes[k]}</div></div>
            <div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => copy(String(hashes![k]))}>{$t("tools.ctf.copy")}</button></div>
          </div>
        {/each}
        <div class="group-row"><div class="group-row-sub">{fmtBytes(hashes.bytes)}</div></div>
      </div></section>
    {/if}
    <section>
      <span class="group-label">{$t("tools.ctf.hmac")}</span>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.ctf.key")}</div>{#if hmacOut}<div class="group-row-sub mono">{hmacOut}</div>{/if}</div>
          <div class="group-row-trailing btn-row">
            <input class="input mono" type="text" bind:value={hmacKey} style:width="9em" />
            <select class="input" bind:value={hmacAlg}><option value="sha256">sha256</option><option value="sha512">sha512</option><option value="sha1">sha1</option><option value="md5">md5</option></select>
            <button class="btn btn-secondary btn-sm" type="button" disabled={!text || !hmacKey} onclick={makeHmac}>{$t("tools.ctf.run")}</button>
          </div>
        </div>
      </div>
    </section>
    <section>
      <span class="group-label">{$t("tools.ctf.identify")}</span>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content"><input class="input mono full" type="text" bind:value={hashInput} placeholder={$t("tools.ctf.paste_hash")} /></div>
          <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={identify}>{$t("tools.ctf.run")}</button></div>
        </div>
        {#each guesses as g (g.name)}
          <div class="group-row"><div class="group-row-content"><div class="group-row-title">{g.name} <span class="tag">{g.confidence} {$t("tools.ctf.confidence")}</span></div><div class="group-row-sub">{g.note}</div></div></div>
        {/each}
      </div>
    </section>
  {/if}

  {#if mode === "magic" && magic}
    <section><div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{magic.guess ? magic.guess.label : $t("tools.ctf.no_type")}</div>
          <div class="group-row-sub">{magic.guess?.mime ?? ""} · {fmtBytes(magic.bytes)} · {$t("tools.ctf.entropy")} {magic.entropy.toFixed(2)}</div>
          {#if magic.extension_mismatch}<div class="group-row-sub"><span class="tag tag-warning">{$t("tools.ctf.mismatch")}</span></div>{/if}
        </div>
      </div>
      {#each magic.embedded as e, i (i)}
        <div class="group-row"><div class="group-row-content"><div class="group-row-title"><span class="tag tag-danger">{$t("tools.ctf.embedded")}</span> {e.label}</div><div class="group-row-sub">{$t("tools.ctf.offset")} {e.offset}</div></div></div>
      {/each}
    </div></section>
    <section>
      <span class="group-label">{$t("tools.ctf.strings")} ({magic.strings.length})</span>
      <div class="group"><div class="group-row block"><pre class="out mono">{magic.strings.join("\n")}</pre></div></div>
    </section>
  {/if}

  {#if mode === "cipher"}
    {#if cipherOut}
      <section><div class="group">
        <div class="group-row block"><pre class="out mono">{cipherOut}</pre></div>
        <div class="group-row"><div class="group-row-content"></div><div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => copy(cipherOut)}>{$t("tools.ctf.copy")}</button></div></div>
      </div></section>
    {/if}
    {#if caesar.length}
      <section><div class="group">
        {#each caesar.slice(0, 8) as c (c.shift)}
          <div class="group-row">
            <div class="group-row-content"><div class="group-row-title">ROT{c.shift} <span class="dim">{$t("tools.ctf.score")} {c.score.toFixed(1)}</span></div><div class="group-row-sub mono">{c.text.slice(0, 160)}</div></div>
            <div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => copy(c.text)}>{$t("tools.ctf.copy")}</button></div>
          </div>
        {/each}
      </div></section>
    {/if}
  {/if}

  {#if mode === "xor" && xorRes}
    {#if xorRes.key_sizes.length}
      <section><div class="group"><div class="group-row"><div class="group-row-content"><div class="group-row-title">{$t("tools.ctf.key_sizes")}</div><div class="group-row-sub mono">{xorRes.key_sizes.map(([s, d]) => `${s} (${d.toFixed(2)})`).join(" · ")}</div></div></div></div></section>
    {/if}
    <section><div class="group">
      {#each xorRes.candidates as c, i (i)}
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.ctf.key")} <span class="mono">{c.key}</span> <span class="dim">0x{c.key_hex}</span></div><div class="group-row-sub mono">{c.text.slice(0, 200)}</div></div>
          <div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => copy(c.text)}>{$t("tools.ctf.copy")}</button></div>
        </div>
      {/each}
    </div></section>
  {/if}

  {#if mode === "encode" && encoded}
    <section><div class="group">
      {#if encoded.error}
        <div class="group-row"><div class="group-row-sub"><span class="tag tag-danger">{$t("tools.common.failed")}</span> {encoded.error}</div></div>
      {:else}
        <div class="group-row block"><pre class="out mono">{encoded.output}</pre></div>
        <div class="group-row"><div class="group-row-content"><div class="group-row-sub">{encoded.bytes} {$t("tools.ctf.bytes")}</div></div><div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => copy(encoded!.output)}>{$t("tools.ctf.copy")}</button></div></div>
      {/if}
    </div></section>
  {/if}

  {#if mode === "freq" && freq}
    <section><div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{freq.looks_like}</div>
          <div class="group-row-sub">{freq.total} {$t("tools.ctf.bytes")} · {freq.distinct} {$t("tools.ctf.distinct")} · {$t("tools.ctf.entropy")} {freq.entropy.toFixed(3)} · {$t("tools.ctf.ioc")} {freq.index_of_coincidence.toFixed(4)}</div>
        </div>
      </div>
      {#each freq.top as e (e.byte)}
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-sub"><span class="mono cell">{e.display}</span> {e.count} · {e.percent.toFixed(2)}%</div>
            <div class="bar" style:width="{Math.max(2, e.percent * 3)}%"></div>
          </div>
        </div>
      {/each}
    </div></section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .dim { color: var(--text-dim); font-weight: 400; }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
  .group-row.block { display: block; }
  .area { width: 100%; resize: vertical; }
  .full { width: 100%; }
  .out { margin: 0; max-height: 22em; overflow: auto; white-space: pre-wrap; }
  .cell { display: inline-block; min-width: 3.5em; }
  .bar { height: 4px; border-radius: 2px; background: var(--accent); margin-top: 4px; }
</style>
