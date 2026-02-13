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
    min-height: calc(100vh - 64px);
    gap: 32px;
  }

  .hero {
    text-align: center;
  }

  .title {
    font-size: 3rem;
    font-weight: 700;
    letter-spacing: -1px;
    background: linear-gradient(135deg, var(--accent), #5dade2);
    -webkit-background-clip: text;
    -webkit-text-fill-color: transparent;
    background-clip: text;
  }

  .subtitle {
    margin-top: 8px;
    color: var(--text-muted);
    font-size: 1.05rem;
  }

  .omnibox-wrapper {
    width: 100%;
    max-width: 560px;
  }

  .omnibox {
    width: 100%;
    padding: 14px 20px;
    font-size: 0.95rem;
    background: #2a2a2a;
    border-radius: 12px;
    color: var(--text);
    border: 1px solid transparent;
    transition: border-color 0.2s, box-shadow 0.2s;
  }

  .omnibox::placeholder {
    color: var(--text-muted);
  }

  .omnibox:focus {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px rgba(46, 134, 193, 0.15);
  }
</style>
