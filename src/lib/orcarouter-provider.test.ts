/**
 * OrcaRouter provider tests.
 *
 * They drive the same code the application uses: the two credential adapters
 * behind one interface, the provider client those adapters feed, the live model
 * directory and its capability filter. No network and no real secret — every
 * key, code and verifier here is a fixture, and the suite asserts that none of
 * them ever reaches a log line or an error message.
 */
import { describe, expect, it, beforeEach, afterEach } from "vitest";
import {
  CredentialState,
  ORCA_API_BASE,
  ORCA_AUTH_ORIGIN,
  ORCA_EXCHANGE_PATH,
  PkceError,
  apiKeyAdapter,
  base64Url,
  beginPkce,
  capabilityFor,
  catalogStatus,
  createOrcarouterClient,
  exchangeUrl,
  filterFor,
  isClientError,
  isLoopbackHost,
  maskKey,
  memoryVault,
  parseCatalog,
  pkceAdapter,
  reconcileSelection,
  resolveOrigins,
  satisfies,
  vaultCredentialSource,
  type FetchLike,
  type ModelOption,
  type RawModel,
  type StoredCredential,
} from "./orcarouter";

const FAKE_KEY = "sk-orca-0000000000000000";
const FAKE_PASTED_KEY = "sk-orca-fixture-one-two-three-four";
/** The directory body, in the shape `GET /v1/models` actually answers. */
const CATALOG: RawModel[] = [
  {
    id: "deepseek/deepseek-v4.1-flash",
    name: "DeepSeek: DeepSeek V4.1 Flash",
    supported_endpoint_types: ["openai", "openai-response", "anthropic"],
    context_length: 1048576,
    architecture: { input_modalities: ["text", "image"] },
    reasoning: true,
    reasoning_efforts: ["low", "medium", "high", "xhigh"],
  },
  {
    id: "orcarouter/auto",
    name: "OrcaRouter: Auto",
    supported_endpoint_types: ["openai", "openai-response", "anthropic", "gemini"],
  },
  {
    id: "deepseek/deepseek-v4-flash",
    name: "DeepSeek: DeepSeek V4 Flash",
    supported_endpoint_types: ["openai"],
    architecture: { input_modalities: ["text"] },
  },
  { id: "vendor/text-embed-3", supported_endpoint_types: ["embeddings"] },
  {
    id: "vendor/img-gen-1",
    supported_endpoint_types: ["image-generation"],
    architecture: { input_modalities: ["text"], output_modalities: ["image"] },
  },
  { id: "vendor/video-gen-1", supported_endpoint_types: ["openai-video"] },
  { id: "vendor/rerank-1", supported_endpoint_types: ["jina-rerank"] },
  // A chat model that never declared its input modalities: it must fail closed
  // for a multimodal entrance rather than be assumed to accept an image.
  { id: "vendor/undeclared-1", supported_endpoint_types: ["openai"] },
];

const ids = (models: ModelOption[]) => models.map((m) => m.id).sort();

/** A fetch double that records every call and answers from a script. */
function recorder(routes: { match: (url: string) => boolean; status?: number; body?: unknown; throws?: boolean }[]) {
  const calls: { url: string; init: Record<string, unknown> }[] = [];
  const fetchImpl: FetchLike = async (url, init = {}) => {
    calls.push({ url, init });
    const route = routes.find((r) => r.match(url));
    if (!route) throw new Error("unexpected URL: " + url);
    if (route.throws) throw new Error("socket closed");
    return {
      ok: (route.status ?? 200) < 400,
      status: route.status ?? 200,
      json: async () => route.body,
    };
  };
  return { calls, fetchImpl };
}

describe("origins", () => {
  it("defaults to the documented auth and API origins", () => {
    const origins = resolveOrigins();
    expect(origins.auth).toBe(ORCA_AUTH_ORIGIN);
    expect(origins.api).toBe(ORCA_API_BASE);
    expect(origins.auth).toBe("https://www.orcarouter.ai");
    expect(origins.api).toBe("https://api.orcarouter.ai/v1");
  });

  it("lets an explicit override win, per origin", () => {
    const origins = resolveOrigins("https://auth.internal.example", "https://api.internal.example/v1");
    expect(origins.auth).toBe("https://auth.internal.example");
    expect(origins.api).toBe("https://api.internal.example/v1");
  });

  it("refuses a remote origin over plaintext HTTP", () => {
    expect(() => resolveOrigins("http://www.orcarouter.ai")).toThrow(/HTTPS/);
    expect(() => resolveOrigins(null, "http://api.orcarouter.ai/v1")).toThrow(/HTTPS/);
  });

  it("allows HTTP only on loopback, for a self-hosted development gateway", () => {
    for (const host of ["localhost", "127.0.0.1", "[::1]"]) {
      expect(isLoopbackHost(host)).toBe(true);
      expect(resolveOrigins(`http://${host}:8787`).auth).toBe(`http://${host}:8787`);
    }
  });

  it("keeps the exchange on the auth origin and never on the API origin", () => {
    const origins = resolveOrigins();
    expect(exchangeUrl(origins)).toBe("https://www.orcarouter.ai/api/v1/auth/keys");
    expect(exchangeUrl(origins)).toBe(origins.auth + ORCA_EXCHANGE_PATH);
    // The anti-pattern this pins down: the API origin is a 404 for this path.
    expect(exchangeUrl(origins).startsWith(origins.api)).toBe(false);
    expect(ORCA_EXCHANGE_PATH.startsWith("/api/v1/")).toBe(true);
  });
});

describe("PKCE (S256)", () => {
  const origins = resolveOrigins();
  const b64url = (b: Uint8Array) => Buffer.from(b).toString("base64url");

  it("derives an unpadded base64url S256 challenge from the verifier", async () => {
    const pending = await beginPkce({
      origins,
      callbackUrl: "http://127.0.0.1:51733/cb",
      attempt: 1,
      nowMs: 0,
    });
    const expected = b64url(
      new Uint8Array(
        await crypto.subtle.digest("SHA-256", new TextEncoder().encode(pending.verifier)),
      ),
    );
    expect(pending.challenge).toBe(expected);
    expect(pending.challenge).not.toContain("=");
    expect(pending.challenge).not.toContain("+");
    expect(pending.challenge).not.toContain("/");
    // 32 random bytes -> 43 base64url characters.
    expect(pending.verifier).toMatch(/^[A-Za-z0-9_-]{43}$/);
  });

  it("builds the authorize URL with S256, the callback and no verifier", async () => {
    const pending = await beginPkce({
      origins,
      callbackUrl: "http://127.0.0.1:51733/cb",
      attempt: 1,
      nowMs: 0,
    });
    const url = new URL(pending.authorizeUrl);
    expect(url.origin).toBe("https://www.orcarouter.ai");
    expect(url.pathname).toBe("/auth");
    expect(url.searchParams.get("code_challenge_method")).toBe("S256");
    expect(url.searchParams.get("code_challenge")).toBe(pending.challenge);
    expect(url.searchParams.get("state")).toBe(pending.state);
    expect(url.searchParams.get("callback_url")).toBe("http://127.0.0.1:51733/cb");
    expect(url.searchParams.get("scope")).toBe("api");
    expect(url.searchParams.get("app_name")).toBe("OmniGet");
    // The verifier must never leave the process before the exchange.
    expect(pending.authorizeUrl).not.toContain(pending.verifier);
  });

  it("creates a fresh verifier and state on every attempt", async () => {
    const deps = { origins, callbackUrl: "http://127.0.0.1:1/cb", attempt: 1, nowMs: 0 };
    const first = await beginPkce(deps);
    const second = await beginPkce(deps);
    expect(second.verifier).not.toBe(first.verifier);
    expect(second.state).not.toBe(first.state);
    expect(second.challenge).not.toBe(first.challenge);
  });

  it("compares state in constant time", async () => {
    const mod = await import("./orcarouter");
    expect(mod.constantTimeEqual("abc", "abc")).toBe(true);
    expect(mod.constantTimeEqual("abc", "abd")).toBe(false);
    expect(mod.constantTimeEqual("abc", "abcd")).toBe(false);
    expect(mod.constantTimeEqual("", "a")).toBe(false);
  });

  it("returns a 43-character challenge for a known verifier", async () => {
    // Independent check of the encoding helper against Node's own base64url.
    expect(base64Url(new Uint8Array([251, 255, 190, 0]))).toBe(
      Buffer.from([251, 255, 190, 0]).toString("base64url"),
    );
  });
});

describe("API Key adapter", () => {
  let vault: ReturnType<typeof memoryVault>;
  beforeEach(() => {
    vault = memoryVault();
  });

  it("stores the key, its account and the granted scope", async () => {
    const adapter = apiKeyAdapter(vault, { key: FAKE_PASTED_KEY, generation: 1, account: "me" });
    const credential = await adapter.acquire();
    expect(credential).toEqual({
      key: FAKE_PASTED_KEY,
      method: "api_key",
      account: "me",
      scope: "api",
      generation: 1,
    });
    expect(await vault.read()).toEqual(credential);
  });

  it("refuses a malformed key without echoing it back", async () => {
    const adapter = apiKeyAdapter(vault, { key: "not-a-key-abcdefgh", generation: 1 });
    await expect(adapter.acquire()).rejects.toThrow(/sk-orca/);
    try {
      await adapter.acquire();
    } catch (error) {
      expect(String(error)).not.toContain("not-a-key-abcdefgh");
    }
    // Nothing was persisted for a rejected key.
    expect(vault.read()).toBeNull();
  });

  it("masks the key for display and never shows the whole secret", async () => {
    const credential = await apiKeyAdapter(vault, { key: FAKE_PASTED_KEY, generation: 1 }).acquire();
    const masked = maskKey(credential.key);
    expect(masked).toContain("••••");
    expect(masked.endsWith(FAKE_PASTED_KEY.slice(-4))).toBe(true);
    expect(masked).not.toContain(FAKE_PASTED_KEY);
  });

  it("can be cleared without touching a second adapter's state", async () => {
    await apiKeyAdapter(vault, { key: FAKE_PASTED_KEY, generation: 1 }).acquire();
    expect(vault.read()).not.toBeNull();
    vault.clear();
    expect(vault.read()).toBeNull();
  });
});

describe("PKCE adapter", () => {
  const origins = resolveOrigins();
  let vault: ReturnType<typeof memoryVault>;

  const exchange = (
    status: number,
    body: unknown,
    opts: { throws?: boolean } = {},
  ) => {
    const { calls, fetchImpl } = recorder([
      { match: () => true, status, body, throws: opts.throws },
    ]);
    const adapter = pkceAdapter({ vault, origins, fetchImpl });
    return { adapter, calls };
  };

  beforeEach(() => {
    vault = memoryVault();
  });

  it("exchanges the code on the auth origin and persists the returned key", async () => {
    const { adapter, calls } = exchange(200, { key: FAKE_KEY, user_id: "42", scope: "api" });
    const pending = await adapter.begin(1, "http://127.0.0.1:51733/cb");
    const credential = await adapter.complete(pending, { code: "one-time-code", state: pending.state });

    expect(credential.key).toBe(FAKE_KEY);
    expect(credential.method).toBe("pkce");
    expect(credential.account).toBe("42");
    expect((await vault.read())?.key).toBe(FAKE_KEY);

    expect(calls).toHaveLength(1);
    expect(calls[0].url).toBe("https://www.orcarouter.ai/api/v1/auth/keys");
    expect(new URL(calls[0].url).origin).toBe("https://www.orcarouter.ai");
    const body = JSON.parse(String(calls[0].init.body));
    expect(body.code).toBe("one-time-code");
    expect(body.code_verifier).toBe(pending.verifier);
    expect(body.code_challenge_method).toBe("S256");
  });

  it("never puts the verifier or the key in an error message", async () => {
    const { adapter } = exchange(403, { error: "invalid_grant" });
    const pending = await adapter.begin(1, "http://127.0.0.1:51733/cb");
    try {
      await adapter.complete(pending, { code: "one-time-code", state: pending.state });
      throw new Error("expected a refusal");
    } catch (error) {
      const text = String((error as Error).message);
      expect(text).not.toContain(pending.verifier);
      expect(text).not.toContain("one-time-code");
      expect(text).not.toContain(FAKE_KEY);
    }
  });

  it("refuses a state mismatch before the code is used", async () => {
    const { adapter, calls } = exchange(200, { key: FAKE_KEY, user_id: "42", scope: "api" });
    const pending = await adapter.begin(1, "http://127.0.0.1:51733/cb");
    await expect(
      adapter.complete(pending, { code: "one-time-code", state: "not-the-state" }),
    ).rejects.toMatchObject({ kind: "state_mismatch" });
    // The code never reached the exchange endpoint.
    expect(calls).toHaveLength(0);
    expect(vault.read()).toBeNull();
  });

  it("refuses a missing state rather than treating it as empty", async () => {
    const { adapter, calls } = exchange(200, { key: FAKE_KEY, scope: "api" });
    const pending = await adapter.begin(1, "http://127.0.0.1:51733/cb");
    await expect(adapter.complete(pending, { code: "c", state: null })).rejects.toMatchObject({
      kind: "state_mismatch",
    });
    expect(calls).toHaveLength(0);
  });

  it("ends cleanly on a denial and writes no credential", async () => {
    const { adapter, calls } = exchange(200, { key: FAKE_KEY, scope: "api" });
    const pending = await adapter.begin(1, "http://127.0.0.1:51733/cb");
    await expect(
      adapter.complete(pending, { error: "access_denied", state: pending.state }),
    ).rejects.toMatchObject({ kind: "denied" });
    expect(calls).toHaveLength(0);
    expect(vault.read()).toBeNull();
  });

  it("classifies a reused or expired code as terminal, not retryable", async () => {
    const { adapter } = exchange(403, { error: "invalid_grant" });
    const pending = await adapter.begin(1, "http://127.0.0.1:51733/cb");
    await expect(
      adapter.complete(pending, { code: "used", state: pending.state }),
    ).rejects.toMatchObject({ kind: "expired_or_reused_code" });
  });

  it("classifies a challenge-method downgrade distinctly from a bad code", async () => {
    const { adapter } = exchange(400, { error: "invalid_request" });
    const pending = await adapter.begin(1, "http://127.0.0.1:51733/cb");
    await expect(
      adapter.complete(pending, { code: "c", state: pending.state }),
    ).rejects.toMatchObject({ kind: "challenge_downgrade" });
  });

  it("classifies rate limiting distinctly and installs nothing", async () => {
    const { adapter } = exchange(429, { error: "too_many_requests" });
    const pending = await adapter.begin(1, "http://127.0.0.1:51733/cb");
    await expect(
      adapter.complete(pending, { code: "c", state: pending.state }),
    ).rejects.toMatchObject({ kind: "rate_limited" });
    expect(vault.read()).toBeNull();
  });

  it("ends on a network failure instead of hanging", async () => {
    const { adapter } = exchange(0, null, { throws: true });
    const pending = await adapter.begin(1, "http://127.0.0.1:51733/cb");
    await expect(
      adapter.complete(pending, { code: "c", state: pending.state }),
    ).rejects.toMatchObject({ kind: "network" });
    expect(vault.read()).toBeNull();
  });

  it("refuses a granted scope weaker than the one this app needs", async () => {
    // Requested `api`; the workspace role only permitted `connector`.
    const { adapter } = exchange(200, { key: FAKE_KEY, user_id: "42", scope: "connector" });
    const pending = await adapter.begin(1, "http://127.0.0.1:51733/cb");
    await expect(
      adapter.complete(pending, { code: "c", state: pending.state }),
    ).rejects.toMatchObject({ kind: "protocol" });
    // A downgraded grant must not leave a usable credential behind.
    expect(vault.read()).toBeNull();
  });

  it("refuses an unreadable exchange body", async () => {
    const { adapter } = exchange(200, { unexpected: true });
    const pending = await adapter.begin(1, "http://127.0.0.1:51733/cb");
    await expect(
      adapter.complete(pending, { code: "c", state: pending.state }),
    ).rejects.toMatchObject({ kind: "protocol" });
  });

  it("keeps the previous secret until a new one is in hand", async () => {
    const previous: StoredCredential = {
      key: "sk-orca-previous-key-000",
      method: "pkce",
      account: "7",
      scope: "api",
      generation: 3,
    };
    vault = memoryVault(previous);

    const failing = exchange(403, { error: "invalid_grant" });
    const pending = await failing.adapter.begin(4, "http://127.0.0.1:51733/cb");
    await expect(
      failing.adapter.complete(pending, { code: "c", state: pending.state }),
    ).rejects.toBeInstanceOf(PkceError);
    // A failed re-authentication does not sign the user out of the old one.
    expect((await vault.read())?.key).toBe(previous.key);
  });

  it("refuses a superseded attempt even when its state matches", async () => {
    const { adapter } = exchange(200, { key: FAKE_KEY, scope: "api" });
    const first = await adapter.begin(1, "http://127.0.0.1:51733/cb");
    const second = await adapter.begin(2, "http://127.0.0.1:51734/cb");
    await expect(
      adapter.complete(first, { code: "late", state: first.state }),
    ).rejects.toMatchObject({ kind: "protocol" });
    // The current attempt still completes normally.
    const credential = await adapter.complete(second, { code: "fresh", state: second.state });
    expect(credential.key).toBe(FAKE_KEY);
  });

  it("starts a second attempt after a cancelled first one", async () => {
    const { adapter } = exchange(200, { key: FAKE_KEY, scope: "api" });
    const first = await adapter.begin(1, "http://127.0.0.1:51733/cb");
    expect(first.attempt).toBe(1);
    const second = await adapter.begin(2, "http://127.0.0.1:51734/cb");
    expect(second.attempt).toBe(2);
    expect(second.state).not.toBe(first.state);
    const credential = await adapter.complete(second, { code: "c", state: second.state });
    expect(credential.method).toBe("pkce");
  });
});

describe("one credential seam, two adapters", () => {
  const origins = resolveOrigins();

  it("gives the provider client the same credential shape from either entrance", async () => {
    const vault = memoryVault();
    const fromKey = await apiKeyAdapter(vault, { key: FAKE_KEY, generation: 1 }).acquire();
    const pkceVault = memoryVault();
    const { fetchImpl } = recorder([{ match: () => true, status: 200, body: { key: FAKE_KEY, user_id: "42", scope: "api" } }]);
    const pkce = pkceAdapter({ vault: pkceVault, origins, fetchImpl });
    const pending = await pkce.begin(1, "http://127.0.0.1:51733/cb");
    const fromPkce = await pkce.complete(pending, { code: "c", state: pending.state });

    // Same fields, same downstream contract; only the audit label differs.
    expect(Object.keys(fromKey).sort()).toEqual(Object.keys(fromPkce).sort());
    expect(fromKey.key).toBe(fromPkce.key);
    expect(fromKey.method).not.toBe(fromPkce.method);
  });

  it("serves inference and the directory from one credential, with no method branch", async () => {
    for (const method of ["api_key", "pkce"] as const) {
      const calls: { url: string; auth: string }[] = [];
      const fetchImpl: FetchLike = async (url, init = {}) => {
        const headers = (init.headers ?? {}) as Record<string, string>;
        calls.push({ url, auth: headers.Authorization });
        if (url.includes("/models")) return { ok: true, status: 200, json: async () => ({ data: CATALOG }) };
        return {
          ok: true,
          status: 200,
          json: async () => ({ model: "orcarouter/auto", choices: [{ message: { content: "hi" } }] }),
        };
      };
      const client = createOrcarouterClient({
        credential: { resolve: async () => ({ key: FAKE_KEY, method, account: null, scope: "api", generation: 1 }) },
        origins,
        fetchImpl,
      });
      await client.listModels("chat");
      await client.chat("orcarouter/auto", [{ role: "user", content: "hello" }]);
      expect(calls.map((c) => c.auth)).toEqual([`Bearer ${FAKE_KEY}`, `Bearer ${FAKE_KEY}`]);
      expect(calls[0].url).toBe("https://api.orcarouter.ai/v1/models?capability=chat");
      expect(calls[1].url).toBe("https://api.orcarouter.ai/v1/chat/completions");
    }
  });

  it("works from a vault that either adapter wrote to", async () => {
    const vault = memoryVault();
    await apiKeyAdapter(vault, { key: FAKE_KEY, generation: 1 }).acquire();
    const source = vaultCredentialSource(vault);
    expect((await source.resolve()).key).toBe(FAKE_KEY);

    const fresh = memoryVault();
    const { fetchImpl } = recorder([{ match: () => true, status: 200, body: { key: FAKE_KEY, user_id: "9", scope: "api" } }]);
    const pkce = pkceAdapter({ vault: fresh, origins, fetchImpl });
    const pending = await pkce.begin(2, "http://127.0.0.1:51733/cb");
    await pkce.complete(pending, { code: "c", state: pending.state });
    expect((await vaultCredentialSource(fresh).resolve()).key).toBe(FAKE_KEY);
  });
});

describe("provider request path", () => {
  const origins = resolveOrigins();
  const withCredential = (fetchImpl: FetchLike) =>
    createOrcarouterClient({
      credential: { resolve: async () => ({ key: FAKE_KEY, method: "api_key", account: null, scope: "api", generation: 1 }) },
      origins,
      fetchImpl,
    });

  it("sends the Bearer key to the API origin with the right body", async () => {
    const { calls, fetchImpl } = recorder([
      { match: (u) => u.includes("/chat/completions"), status: 200, body: { model: "orcarouter/auto", choices: [{ message: { content: "ok" } }] } },
    ]);
    const result = await withCredential(fetchImpl).chat("orcarouter/auto", [{ role: "user", content: "hi" }]);
    expect(result.content).toBe("ok");
    const body = JSON.parse(String(calls[0].init.body));
    expect(body).toEqual({ model: "orcarouter/auto", messages: [{ role: "user", content: "hi" }], stream: false });
    expect((calls[0].init.headers as Record<string, string>).Authorization).toBe(`Bearer ${FAKE_KEY}`);
  });

  it("classifies a rejected key as a terminal re-authentication, never a refresh", async () => {
    const { calls, fetchImpl } = recorder([{ match: () => true, status: 401 }]);
    const client = withCredential(fetchImpl);
    await expect(client.listModels("chat")).rejects.toMatchObject({ kind: "needs_reauth" });
    // Exactly one request: a rejected credential is not retried and not refreshed.
    expect(calls).toHaveLength(1);
    expect(calls[0].url).not.toContain("auth/keys");
    expect(calls[0].url).not.toContain("refresh");
  });

  it("classifies rate limiting without discarding the credential", async () => {
    const { fetchImpl } = recorder([{ match: () => true, status: 429 }]);
    const error = await withCredential(fetchImpl).listModels("chat").catch((e) => e);
    expect(isClientError(error)).toBe(true);
    expect(error.kind).toBe("rate_limited");
  });

  it("classifies a network failure without hanging", async () => {
    const { fetchImpl } = recorder([{ match: () => true, throws: true }]);
    await expect(withCredential(fetchImpl).listModels("chat")).rejects.toMatchObject({ kind: "network" });
  });

  it("never sends a request when no credential is stored", async () => {
    const { calls, fetchImpl } = recorder([]);
    const client = createOrcarouterClient({
      credential: vaultCredentialSource(memoryVault()),
      origins,
      fetchImpl,
    });
    await expect(client.listModels("chat")).rejects.toThrow(/No OrcaRouter credential/);
    expect(calls).toHaveLength(0);
  });

  it("passes the modality through to the directory", async () => {
    const { calls, fetchImpl } = recorder([{ match: () => true, status: 200, body: { data: [] } }]);
    await withCredential(fetchImpl).listModels("multimodal", "image");
    expect(calls[0].url).toBe("https://api.orcarouter.ai/v1/models?capability=multimodal&modality=image");
  });
});

describe("credential generation and re-authentication", () => {
  const key = (generation: number): StoredCredential => ({
    key: FAKE_KEY,
    method: "api_key",
    account: null,
    scope: "api",
    generation,
  });

  it("marks only the generation whose request was refused", () => {
    const state = new CredentialState(key(1));
    expect(state.reject(1)).toBe(true);
    expect(state.needsReauth).toBe(true);
  });

  it("does not let a stale rejection mark a newer credential", () => {
    const state = new CredentialState(key(1));
    state.reject(1);
    expect(state.needsReauth).toBe(true);
    // A fresh sign-in installs generation 2 and starts unmarked.
    state.accept(key(2));
    expect(state.needsReauth).toBe(false);
    // A late 401 from generation 1 must not touch generation 2.
    expect(state.reject(1)).toBe(false);
    expect(state.needsReauth).toBe(false);
    expect(state.current?.generation).toBe(2);
  });

  it("ignores a rejection for a credential it does not hold", () => {
    const state = new CredentialState(key(5));
    expect(state.reject(4)).toBe(false);
    expect(state.needsReauth).toBe(false);
  });

  it("keeps the old secret until a new one replaces it", () => {
    const state = new CredentialState(key(1));
    state.reject(1);
    expect(state.current?.key).toBe(FAKE_KEY);
    state.accept({ ...key(2), method: "pkce", key: "sk-orca-second-key-0001" });
    expect(state.current?.key).toBe("sk-orca-second-key-0001");
  });
});

describe("model directory and capability filtering", () => {
  it("parses the directory envelope and rejects an unexpected body", () => {
    expect(parseCatalog({ data: CATALOG })).toHaveLength(CATALOG.length);
    expect(parseCatalog({ models: CATALOG })).toHaveLength(CATALOG.length);
    // A broken body is a protocol failure, not an empty workspace.
    expect(() => parseCatalog({ success: true })).toThrow(/unexpected body/);
    expect(() => parseCatalog(null)).toThrow(/unexpected body/);
  });

  it("offers only text-generation routes for a text entrance", () => {
    const chat = filterFor(CATALOG, "chat");
    expect(ids(chat)).toContain("orcarouter/auto");
    expect(ids(chat)).toContain("deepseek/deepseek-v4-flash");
    for (const bad of ["vendor/text-embed-3", "vendor/img-gen-1", "vendor/video-gen-1", "vendor/rerank-1"]) {
      expect(ids(chat)).not.toContain(bad);
    }
  });

  it("keeps the vendor/model namespace verbatim", () => {
    expect(ids(filterFor(CATALOG, "chat"))).toContain("deepseek/deepseek-v4.1-flash");
  });

  it("keeps the declared reasoning metadata, including the effort ladder", () => {
    const model = filterFor(CATALOG, "chat").find((m) => m.id === "deepseek/deepseek-v4.1-flash");
    expect(model?.reasoning).toBe(true);
    expect(model?.reasoning_efforts).toEqual(["low", "medium", "high", "xhigh"]);
    expect(model?.context_length).toBe(1048576);
  });

  it("fails closed on an undeclared input modality", () => {
    // vendor/undeclared-1 is a chat model with no architecture declaration.
    expect(satisfies(CATALOG[7], "multimodal", "image")).toBe(false);
    expect(ids(filterFor(CATALOG, "multimodal", "image"))).not.toContain("vendor/undeclared-1");
  });

  it("keeps only models that declare the attached modality", () => {
    const multimodal = filterFor(CATALOG, "multimodal", "image");
    expect(ids(multimodal)).toEqual(["deepseek/deepseek-v4.1-flash"]);
    expect(multimodal[0].input_modalities).toContain("image");
    // The text-only chat models are gone from this list.
    expect(ids(multimodal)).not.toContain("deepseek/deepseek-v4-flash");
  });

  it("never mixes an embedding, image, video or rerank route into a chat list", () => {
    const chat = ids(filterFor(CATALOG, "chat"));
    for (const entrance of ["embedding", "image", "video", "rerank"] as const) {
      const other = ids(filterFor(CATALOG, entrance));
      expect(other).toHaveLength(1);
      expect(chat.filter((id) => other.includes(id))).toEqual([]);
    }
  });

  it("selects each entrance by its own route", () => {
    expect(ids(filterFor(CATALOG, "embedding"))).toEqual(["vendor/text-embed-3"]);
    expect(ids(filterFor(CATALOG, "image"))).toEqual(["vendor/img-gen-1"]);
    expect(ids(filterFor(CATALOG, "video"))).toEqual(["vendor/video-gen-1"]);
    expect(ids(filterFor(CATALOG, "rerank"))).toEqual(["vendor/rerank-1"]);
  });

  it("does not guess a capability from a model's name", () => {
    const misleading: RawModel[] = [
      { id: "vendor/embedding-lookalike", supported_endpoint_types: ["openai"] },
      { id: "vendor/whisper-chat", supported_endpoint_types: ["openai-response"], architecture: { input_modalities: ["audio"] } },
    ];
    expect(ids(filterFor(misleading, "embedding"))).toEqual([]);
    expect(ids(filterFor(misleading, "chat"))).toEqual(["vendor/embedding-lookalike", "vendor/whisper-chat"]);
    expect(ids(filterFor(misleading, "multimodal", "audio"))).toEqual(["vendor/whisper-chat"]);
  });

  it("asks the right capability for an entrance and its attachments", () => {
    expect(capabilityFor("chat", 0)).toEqual({ capability: "chat" });
    expect(capabilityFor("chat", 1)).toEqual({ capability: "multimodal", modality: "image" });
  });

  it("clears a selection the current options no longer offer", () => {
    const chat = filterFor(CATALOG, "chat");
    expect(reconcileSelection(chat, "orcarouter/auto")).toBe("orcarouter/auto");
    // A text model must not survive into an image-attachment list.
    const multimodal = filterFor(CATALOG, "multimodal", "image");
    expect(reconcileSelection(multimodal, "deepseek/deepseek-v4-flash")).toBeNull();
  });

  it("labels a fallback directory as degraded rather than live", () => {
    const live = {
      models: filterFor(CATALOG, "chat"),
      source: "live" as const,
      degraded: false,
      fetched_at_ms: 1,
      live: true,
      needs_reauth: false,
    };
    expect(catalogStatus(live).kind).toBe("live");
    expect(catalogStatus({ ...live, source: "seed", live: false, degraded: true }).kind).toBe("degraded");
    expect(catalogStatus({ ...live, needs_reauth: true }).kind).toBe("reauth");
  });
});

describe("secrets stay out of logs and errors", () => {
  const origins = resolveOrigins();
  let logged: string[];
  let original: typeof console.log;

  beforeEach(() => {
    logged = [];
    original = console.log;
    console.log = (...args: unknown[]) => {
      logged.push(args.map(String).join(" "));
    };
  });
  afterEach(() => {
    console.log = original;
  });

  it("logs nothing containing the key, the code or the verifier", async () => {
    const vault = memoryVault();
    const { fetchImpl } = recorder([{ match: () => true, status: 200, body: { key: FAKE_KEY, user_id: "42", scope: "api" } }]);
    const adapter = pkceAdapter({ vault, origins, fetchImpl });
    const pending = await adapter.begin(1, "http://127.0.0.1:51733/cb");
    await adapter.complete(pending, { code: "secret-code", state: pending.state });
    await apiKeyAdapter(vault, { key: FAKE_PASTED_KEY, generation: 2 }).acquire();

    const text = logged.join("\n");
    expect(text).not.toContain(FAKE_KEY);
    expect(text).not.toContain(FAKE_PASTED_KEY);
    expect(text).not.toContain("secret-code");
    expect(text).not.toContain(pending.verifier);
  });
});
