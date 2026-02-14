<script lang="ts">
  let url = $state("");
</script>

<div class="home">
  <div class="hero">
    <h1 class="title">OmniGet</h1>
    <p class="subtitle">Native downloads, no external dependencies</p>
  </div>

  <div class="omnibox-wrapper">
    <input
      class="omnibox"
      type="text"
      placeholder="Paste a URL to get started"
      bind:value={url}
    />
  </div>
</div>

<style>
  .home {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    min-height: calc(100vh - var(--padding) * 4);
    gap: calc(var(--padding) * 2);
  }

  .hero {
    text-align: center;
  }

  .title {
    font-size: 32px;
    font-weight: 500;
    letter-spacing: -1px;
    background: linear-gradient(135deg, var(--blue), #5dade2);
    -webkit-background-clip: text;
    -webkit-text-fill-color: transparent;
    background-clip: text;
  }

  .subtitle {
    margin-top: calc(var(--padding) / 2);
    color: var(--gray);
    font-size: 14.5px;
  }

  .omnibox-wrapper {
    width: 100%;
    max-width: 560px;
  }

  .omnibox {
    width: 100%;
    padding: var(--padding) calc(var(--padding) + 4px);
    font-size: 14.5px;
    background: var(--button);
    border-radius: var(--border-radius);
    color: var(--secondary);
    border: 1px solid var(--input-border);
  }

  .omnibox::placeholder {
    color: var(--gray);
  }

  .omnibox:focus-visible {
    border-color: var(--secondary);
    outline: none;
  }
</style>
