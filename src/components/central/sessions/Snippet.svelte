<script lang="ts">
  /** Trecho de busca com o termo marcado (posições em caracteres). */
  interface Props {
    text: string;
    start: number;
    len: number;
  }
  let { text, start, len }: Props = $props();
  let chars = $derived(Array.from(text));
  let a = $derived(chars.slice(0, start).join(""));
  let b = $derived(chars.slice(start, start + len).join(""));
  let c = $derived(chars.slice(start + len).join(""));
</script>

<span class="snip">{a}<mark>{b}</mark>{c}</span>

<style>
  .snip {
    overflow-wrap: anywhere;
    color: var(--text-muted);
  }
  mark {
    background: var(--selection);
    color: var(--text);
    border-radius: 2px;
    padding: 0 1px;
  }
</style>
