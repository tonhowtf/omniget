<script lang="ts">
  const themes = [
    "dark", "light", "catppuccin-mocha", "catppuccin-macchiato", "catppuccin-frappe",
    "catppuccin-latte", "one-dark-pro", "dracula", "nyxvamp-veil", "nyxvamp-obsidian",
    "nyxvamp-radiance", "eink-day", "eink-sepia", "eink-night",
  ];

  let currentTheme = $state("dark");
  let toggleOn = $state(true);
  let toggleOff = $state(false);

  function setTheme(t: string) {
    currentTheme = t;
    document.documentElement.setAttribute("data-theme", t);
  }
</script>

<div class="sink">
  <header class="sink-head">
    <h1 class="page-title">Kitchen sink</h1>
    <p class="sink-sub">Every primitive, every state. Dev-only audit surface — not linked from nav.</p>
    <div class="sink-themes">
      {#each themes as t}
        <button class="btn btn-sm {currentTheme === t ? 'btn-primary' : 'btn-secondary'}" onclick={() => setTheme(t)}>{t}</button>
      {/each}
    </div>
  </header>

  <section class="sink-section">
    <div class="section-header"><span class="section-header-title">Buttons</span></div>
    <div class="sink-grid">
      <div class="sink-cell"><span class="sink-label">primary</span><button class="btn btn-primary">Download in 1080p</button></div>
      <div class="sink-cell"><span class="sink-label">secondary</span><button class="btn btn-secondary">Open folder</button></div>
      <div class="sink-cell"><span class="sink-label">ghost</span><button class="btn btn-ghost">Cancel</button></div>
      <div class="sink-cell"><span class="sink-label">destructive</span><button class="btn btn-destructive">Remove 3 items</button></div>
      <div class="sink-cell"><span class="sink-label">pill</span><button class="btn btn-pill">Video</button></div>
      <div class="sink-cell"><span class="sink-label">pill active</span><button class="btn btn-pill active">Audio</button></div>
      <div class="sink-cell"><span class="sink-label">sm</span><button class="btn btn-sm btn-secondary">Retry</button></div>
      <div class="sink-cell"><span class="sink-label">lg</span><button class="btn btn-lg btn-primary">Get started</button></div>
      <div class="sink-cell"><span class="sink-label">disabled</span><button class="btn btn-primary" disabled>Download</button></div>
      <div class="sink-cell"><span class="sink-label">loading</span><button class="btn btn-primary loading"><span class="spinner"></span>Preparing…</button></div>
    </div>
  </section>

  <section class="sink-section">
    <div class="section-header"><span class="section-header-title">Inputs</span></div>
    <div class="sink-grid">
      <div class="sink-cell wide">
        <div class="field">
          <label class="field-label" for="ks-in-1">URL</label>
          <input id="ks-in-1" class="input" placeholder="Paste a link…" />
          <span class="field-hint">YouTube, Instagram and 1000+ sites.</span>
        </div>
      </div>
      <div class="sink-cell wide">
        <div class="field">
          <label class="field-label" for="ks-in-2">Output folder</label>
          <input id="ks-in-2" class="input error" value="/nonexistent/path" />
          <span class="field-error">This folder does not exist. Pick another one.</span>
        </div>
      </div>
      <div class="sink-cell wide">
        <div class="field">
          <label class="field-label" for="ks-in-3">Disabled</label>
          <input id="ks-in-3" class="input" disabled value="Managed by cookies file" />
        </div>
      </div>
      <div class="sink-cell wide">
        <div class="field">
          <label class="field-label" for="ks-in-4">Large</label>
          <input id="ks-in-4" class="input input-lg" placeholder="Search settings…" />
        </div>
      </div>
      <div class="sink-cell wide">
        <div class="field">
          <label class="field-label" for="ks-ta">Notes</label>
          <textarea id="ks-ta" class="input" placeholder="Anything worth remembering…"></textarea>
        </div>
      </div>
    </div>
  </section>

  <section class="sink-section">
    <div class="section-header"><span class="section-header-title">Select / picker</span></div>
    <div class="sink-grid">
      <div class="sink-cell"><span class="sink-label">select</span>
        <select class="select">
          <option>1080p</option><option>720p</option><option>480p</option>
        </select>
      </div>
      <div class="sink-cell"><span class="sink-label">disabled</span>
        <select class="select" disabled><option>mp4</option></select>
      </div>
    </div>
  </section>

  <section class="sink-section">
    <div class="section-header"><span class="section-header-title">Toggle / checkbox / radio</span></div>
    <div class="sink-grid">
      <div class="sink-cell"><span class="sink-label">toggle on</span>
        <button class="toggle" class:on={toggleOn} onclick={() => (toggleOn = !toggleOn)} role="switch" aria-checked={toggleOn} aria-label="Example toggle on"><span class="toggle-knob"></span></button>
      </div>
      <div class="sink-cell"><span class="sink-label">toggle off</span>
        <button class="toggle" class:on={toggleOff} onclick={() => (toggleOff = !toggleOff)} role="switch" aria-checked={toggleOff} aria-label="Example toggle off"><span class="toggle-knob"></span></button>
      </div>
      <div class="sink-cell"><span class="sink-label">checkbox</span>
        <label class="sink-inline"><input type="checkbox" class="checkbox" checked /> Embed thumbnail</label>
      </div>
      <div class="sink-cell"><span class="sink-label">unchecked</span>
        <label class="sink-inline"><input type="checkbox" class="checkbox" /> Skip existing</label>
      </div>
      <div class="sink-cell"><span class="sink-label">disabled</span>
        <label class="sink-inline"><input type="checkbox" class="checkbox" disabled /> Unavailable</label>
      </div>
      <div class="sink-cell"><span class="sink-label">radio</span>
        <label class="sink-inline"><input type="radio" class="radio" name="ks-r" checked /> Full names</label>
      </div>
      <div class="sink-cell"><span class="sink-label">radio 2</span>
        <label class="sink-inline"><input type="radio" class="radio" name="ks-r" /> Usernames</label>
      </div>
    </div>
  </section>

  <section class="sink-section">
    <div class="section-header"><span class="section-header-title">Badge / tag / avatar</span></div>
    <div class="sink-grid">
      <div class="sink-cell"><span class="sink-label">badge</span><span class="badge">3</span></div>
      <div class="sink-cell"><span class="sink-label">badge 99+</span><span class="badge">99+</span></div>
      <div class="sink-cell"><span class="sink-label">neutral</span><span class="badge badge-neutral">12</span></div>
      <div class="sink-cell"><span class="sink-label">tag</span><span class="tag">yt-dlp</span></div>
      <div class="sink-cell"><span class="sink-label">accent</span><span class="tag tag-accent">Installed</span></div>
      <div class="sink-cell"><span class="sink-label">success</span><span class="tag tag-success">Complete</span></div>
      <div class="sink-cell"><span class="sink-label">warning</span><span class="tag tag-warning">Stalled</span></div>
      <div class="sink-cell"><span class="sink-label">danger</span><span class="tag tag-danger">Failed</span></div>
      <div class="sink-cell"><span class="sink-label">avatar</span><span class="avatar">TW</span></div>
      <div class="sink-cell"><span class="sink-label">sm / lg</span><span class="avatar avatar-sm">A</span> <span class="avatar avatar-lg">TW</span></div>
    </div>
  </section>

  <section class="sink-section">
    <div class="section-header"><span class="section-header-title">Card / list rows / separator</span></div>
    <div class="surface-card sink-card-demo">
      <div class="list-row interactive" role="button" tabindex="0">
        <span class="avatar">YT</span>
        <span class="list-row-content">
          <span class="list-row-title">Building a Desktop App with Tauri 2 — Full Walkthrough</span>
          <span class="list-row-sub">youtube.com · 700 MB · 1 hour ago</span>
        </span>
        <span class="list-row-trailing"><span class="tag tag-success">Complete</span></span>
      </div>
      <hr class="separator inset" />
      <div class="list-row interactive selected" role="button" tabindex="0">
        <span class="avatar">IG</span>
        <span class="list-row-content">
          <span class="list-row-title">Selected row</span>
          <span class="list-row-sub">instagram.com · waiting</span>
        </span>
        <span class="list-row-trailing">⌘⏎</span>
      </div>
      <hr class="separator inset" />
      <div class="list-row compact">
        <span class="list-row-content"><span class="list-row-title">Compact row (28px)</span></span>
        <span class="list-row-trailing"><span class="badge badge-neutral">2</span></span>
      </div>
    </div>
  </section>

  <section class="sink-section">
    <div class="section-header">
      <span class="section-header-title">Section header</span>
      <button class="section-header-action">Show all</button>
    </div>
    <p class="sink-note">This very header demonstrates the primitive (caps title + optional action).</p>
  </section>

  <section class="sink-section">
    <div class="section-header"><span class="section-header-title">Tooltip / menu / popover</span></div>
    <div class="sink-grid">
      <div class="sink-cell wide">
        <div class="sink-static-wrap">
          <span class="tooltip sink-static">Copies the file path without opening the folder</span>
        </div>
      </div>
      <div class="sink-cell wide">
        <div class="popover sink-static">
          <button class="menu-item">Open file</button>
          <button class="menu-item">Reveal in Finder</button>
          <button class="menu-item" disabled>Re-encode…</button>
          <div class="menu-separator"></div>
          <button class="menu-item destructive">Remove from history</button>
        </div>
      </div>
    </div>
  </section>

  <section class="sink-section">
    <div class="section-header"><span class="section-header-title">Dialog</span></div>
    <div class="dialog-card sink-static">
      <h2 class="dialog-title">Remove 3 downloads from history?</h2>
      <p class="dialog-body">The files stay on disk. Only the history entries are removed. You can undo this from the toast.</p>
      <div class="dialog-actions">
        <button class="btn btn-secondary">Keep</button>
        <button class="btn btn-destructive">Remove 3 downloads</button>
      </div>
    </div>
  </section>

  <section class="sink-section">
    <div class="section-header"><span class="section-header-title">Toast</span></div>
    <div class="sink-col">
      <div class="toast-card">Link copied to the clipboard.</div>
      <div class="toast-card success">Download complete — Conference Keynote 2026.</div>
      <div class="toast-card warning">Download stalled. Retrying in 5 s…</div>
      <div class="toast-card error">The site asked for a login. Import your cookies in Settings → Cookies.</div>
    </div>
  </section>

  <section class="sink-section">
    <div class="section-header"><span class="section-header-title">Tabs / segmented</span></div>
    <div class="segmented">
      <button class="segmented-btn active">URL</button>
      <button class="segmented-btn">Batch</button>
      <button class="segmented-btn">Torrent</button>
      <button class="segmented-btn">P2P</button>
      <button class="segmented-btn" disabled>Soon</button>
    </div>
  </section>

  <section class="sink-section">
    <div class="section-header"><span class="section-header-title">Skeleton / loading</span></div>
    <div class="sink-col">
      <div class="sink-skel-row">
        <div class="skeleton circle" style="width:28px;height:28px"></div>
        <div class="sink-skel-lines">
          <div class="skeleton" style="width:60%;height:12px"></div>
          <div class="skeleton" style="width:35%;height:10px"></div>
        </div>
      </div>
      <div class="sink-inline"><span class="spinner"></span><span class="sink-note">Detecting platform…</span></div>
    </div>
  </section>

  <section class="sink-section">
    <div class="section-header"><span class="section-header-title">Empty state</span></div>
    <div class="surface-card">
      <div class="empty-state">
        <svg class="empty-state-icon" width="40" height="40" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M12 3v12m0 0l-4-4m4 4l4-4M4 17v2a2 2 0 002 2h12a2 2 0 002-2v-2"/></svg>
        <span class="empty-state-title">No downloads yet</span>
        <span class="empty-state-hint">Paste a link on the home screen — the queue and history will show up here.</span>
        <button class="btn btn-primary">Paste a link</button>
      </div>
    </div>
  </section>

  <section class="sink-section">
    <div class="section-header"><span class="section-header-title">Progress</span></div>
    <div class="sink-col">
      <div class="progress"><div class="progress-fill" style="width:42%"></div></div>
      <div class="progress"><div class="progress-fill success" style="width:100%"></div></div>
      <div class="progress"><div class="progress-fill paused" style="width:63%"></div></div>
      <div class="progress"><div class="progress-fill danger" style="width:18%"></div></div>
      <div class="progress indeterminate"><div class="progress-fill"></div></div>
    </div>
  </section>

  <section class="sink-section">
    <div class="section-header"><span class="section-header-title">Kbd</span></div>
    <p class="sink-note">Press <span class="kbd">⌘</span> <span class="kbd">K</span> to open the palette, <span class="kbd">⌘⇧D</span> to grab the clipboard.</p>
  </section>
</div>

<style>
  .sink {
    max-width: 860px;
    margin: 0 auto;
    padding: var(--space-5) var(--space-4) var(--space-8);
    display: flex;
    flex-direction: column;
    gap: var(--space-6);
  }

  .sink-head {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .sink-sub {
    font-size: var(--text-base);
    color: var(--text-dim);
    margin: 0;
  }

  .sink-themes {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
    margin-top: var(--space-2);
  }

  .sink-section {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .sink-grid {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
  }

  .sink-cell {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--space-1);
    min-width: 120px;
  }

  .sink-cell.wide {
    min-width: 280px;
    flex: 1;
  }

  .sink-label {
    font-size: var(--text-caption);
    font-weight: 600;
    letter-spacing: var(--track-caps);
    text-transform: uppercase;
    color: var(--text-faint);
  }

  .sink-inline,
  .sink-note {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-base);
    color: var(--text-muted);
    margin: 0;
  }

  .sink-col {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    max-width: 440px;
  }

  .sink-card-demo {
    padding: var(--space-1);
  }

  .sink-static {
    position: static;
    pointer-events: auto;
  }

  .sink-static-wrap {
    position: relative;
    min-height: 32px;
  }

  .sink-skel-row {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }

  .sink-skel-lines {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
</style>
