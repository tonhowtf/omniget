<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";

  let email = $state("");
  let password = $state("");
  let loading = $state(false);
  let error = $state("");
  let loggedIn = $state(false);

  async function handleLogin() {
    error = "";
    loading = true;
    try {
      await invoke("hotmart_login", { email, password });
      loggedIn = true;
    } catch (e: any) {
      error = typeof e === "string" ? e : e.message ?? "Erro desconhecido";
    } finally {
      loading = false;
    }
  }
</script>

<div class="hotmart">
  <h1 class="page-title">Hotmart</h1>

  {#if loggedIn}
    <div class="card success-card">
      <span class="success-icon">&#10003;</span>
      <p class="success-text">Logado!</p>
    </div>

    <div class="card empty">
      <p class="empty-text">Your courses will appear here</p>
    </div>
  {:else}
    <div class="card">
      <h2 class="card-title">Login</h2>
      <form class="form" onsubmit={(e) => { e.preventDefault(); handleLogin(); }}>
        <label class="field">
          <span class="label">Email</span>
          <input
            type="email"
            placeholder="you@example.com"
            bind:value={email}
            class="input"
            disabled={loading}
            required
          />
        </label>
        <label class="field">
          <span class="label">Password</span>
          <input
            type="password"
            placeholder="Your password"
            bind:value={password}
            class="input"
            disabled={loading}
            required
          />
        </label>

        {#if error}
          <p class="error-msg">{error}</p>
        {/if}

        <button type="submit" class="btn" disabled={loading}>
          {#if loading}
            Autenticando...
          {:else}
            Sign in
          {/if}
        </button>
      </form>
    </div>
  {/if}
</div>

<style>
  .hotmart {
    max-width: 480px;
  }

  .page-title {
    font-size: 1.5rem;
    font-weight: 600;
    margin-bottom: 24px;
  }

  .card {
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 12px;
    padding: 24px;
    margin-bottom: 16px;
  }

  .card-title {
    font-size: 1rem;
    font-weight: 500;
    margin-bottom: 20px;
  }

  .form {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .label {
    font-size: 0.85rem;
    color: var(--text-muted);
  }

  .input {
    width: 100%;
    padding: 10px 14px;
    font-size: 0.9rem;
    background: #2a2a2a;
    border-radius: 8px;
    color: var(--text);
    border: 1px solid var(--border);
    transition: border-color 0.2s;
  }

  .input::placeholder {
    color: var(--text-muted);
  }

  .input:focus {
    border-color: var(--accent);
  }

  .input:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .error-msg {
    color: #ef4444;
    font-size: 0.85rem;
  }

  .btn {
    padding: 10px 20px;
    font-size: 0.9rem;
    font-weight: 500;
    background: var(--accent);
    color: #fff;
    border-radius: 8px;
    transition: background-color 0.2s;
    margin-top: 4px;
  }

  .btn:hover:not(:disabled) {
    background: var(--accent-hover);
  }

  .btn:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }

  .success-card {
    display: flex;
    align-items: center;
    gap: 12px;
  }

  .success-icon {
    font-size: 1.5rem;
    color: #22c55e;
  }

  .success-text {
    font-size: 1rem;
    font-weight: 500;
    color: #22c55e;
  }

  .empty {
    display: flex;
    align-items: center;
    justify-content: center;
    min-height: 120px;
  }

  .empty-text {
    color: var(--text-muted);
    font-size: 0.9rem;
  }
</style>
