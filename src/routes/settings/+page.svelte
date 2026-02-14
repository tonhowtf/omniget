<div class="settings">
  <h1>Settings</h1>

  <section class="section">
    <h5 class="section-title">Appearance</h5>
    <div class="card">
      <p class="placeholder">Theme and display preferences</p>
    </div>
  </section>

  <section class="section">
    <h5 class="section-title">Download</h5>
    <div class="card">
      <p class="placeholder">Download path and behavior settings</p>
    </div>
  </section>

  <section class="section">
    <h5 class="section-title">Hotmart</h5>
    <div class="card">
      <p class="placeholder">Account and sync settings</p>
    </div>
  </section>

  <section class="section">
    <h5 class="section-title">Advanced</h5>
    <div class="card">
      <p class="placeholder">Concurrent downloads, cache, and logs</p>
    </div>
  </section>
</div>

<style>
  .settings {
    display: flex;
    flex-direction: column;
    align-items: center;
    min-height: calc(100vh - var(--padding) * 4);
    padding-top: calc(var(--padding) * 2);
    gap: calc(var(--padding) * 1.5);
  }

  .settings > :global(*) {
    width: 100%;
    max-width: 560px;
  }

  .section {
    display: flex;
    flex-direction: column;
    gap: calc(var(--padding) / 2);
  }

  .section-title {
    color: var(--gray);
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }

  .card {
    background: var(--button-elevated);
    box-shadow: var(--button-box-shadow);
    border-radius: var(--border-radius);
    padding: calc(var(--padding) + 4px);
  }

  .placeholder {
    color: var(--gray);
    font-size: 14.5px;
  }
</style>
