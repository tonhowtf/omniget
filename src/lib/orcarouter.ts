/**
 * Client-side helpers for the OrcaRouter model selector.
 *
 * The catalog and its capability filtering live in Rust
 * (`omniget_core::core::orcarouter`), which is the single source of truth: the
 * API key never reaches this process, so the filtering cannot live here
 * without leaking it. What belongs on this side is the part only the UI knows —
 * which entrance is asking, and what the user has actually attached.
 *
 * These helpers are pure so they can be tested without a running Tauri host.
 */

/** An entrance that consumes models. Mirrors the Rust `ModelCapability`. */
export type CapabilityRequest = {
  /** The `capability` argument sent to `orcarouter_models`. */
  capability: "chat" | "multimodal" | "embedding" | "image" | "video" | "rerank";
  /** The `modality` argument, only for a multimodal entrance. */
  modality?: "image" | "audio" | "video";
};

/**
 * The non-text modalities a given AI entrance actually uploads.
 *
 * This repository has no entrance that sends image, audio or video content to a
 * model: every AI call site (`humanize`, subtitle translation, video summarise,
 * the league coach, the ffmpeg proposal) sends text. `0` is therefore the
 * honest mapping for the entrances that exist, and the multimodal branch is
 * reached only if an entrance that attaches media is added.
 */
export const ENTRANCE_ATTACHMENTS: Record<string, number> = {
  chat: 0,
};

/** Options as returned by the backend. */
export type ModelOption = {
  id: string;
  name: string;
  context_length: number | null;
  max_completion_tokens: number | null;
  input_modalities: string[];
  /**
   * The routes the catalog advertises for this model. The backend filters by
   * them; they are carried across the IPC boundary so the selector's options can
   * be re-derived here from the same metadata instead of being trusted blindly.
   */
  endpoint_types?: string[];
  reasoning: boolean;
  reasoning_efforts: string[];
};

export type CatalogSource = "live" | "last_known_good" | "seed";

export type CatalogView = {
  models: ModelOption[];
  source: CatalogSource;
  degraded: boolean;
  fetched_at_ms: number;
  live: boolean;
  needs_reauth: boolean;
};

/**
 * Build the capability request for an entrance given how many non-text
 * attachments the user has added.
 *
 * An entrance with no attachment asks for plain chat. An entrance with an
 * attachment asks for chat restricted to that modality, so an undeclared model
 * can never appear in the list.
 */
export function capabilityFor(entrance: string, attachments: number): CapabilityRequest {
  if (attachments <= 0) return { capability: "chat" };
  const modality = entrance === "chat" ? "image" : "image";
  return { capability: "multimodal", modality };
}

/**
 * Whether a previously selected model may stay selected.
 *
 * Returns the id when it is still present in the freshly resolved options, and
 * `null` when it is not — the caller must clear the selection and prompt the
 * user rather than keep a value the current entrance cannot use.
 */
export function reconcileSelection(
  options: ModelOption[],
  selected: string | null | undefined,
): string | null {
  const want = (selected ?? "").trim();
  if (!want) return null;
  return options.some((m) => m.id === want) ? want : null;
}

/** How the catalog state should be described to the user. */
export type CatalogStatus =
  | { kind: "live"; labelKey: string }
  | { kind: "degraded"; labelKey: string }
  | { kind: "reauth"; labelKey: string };

/**
 * Describe a catalog response. A degraded catalog is always labelled, so a
 * fallback list is never presented as if it were the live directory.
 */
export function catalogStatus(view: CatalogView | null): CatalogStatus {
  if (view?.needs_reauth) return { kind: "reauth", labelKey: "settings.ai.orca_status_reauth" };
  if (!view || !view.live) {
    return { kind: "degraded", labelKey: "settings.ai.orca_status_degraded" };
  }
  return { kind: "live", labelKey: "settings.ai.orca_status_live" };
}

/**
 * Whether the selector can offer a real choice.
 *
 * An empty list is a real state (the workspace has no model for this entrance)
 * and must be shown as such. It must never fall back to a free-text field: a
 * typed model id cannot be checked against the catalog, and the campaign's
 * rule is that the options come from the API.
 */
export function isSelectable(view: CatalogView | null): boolean {
  return !!view && view.models.length > 0;
}

/* ------------------------------------------------------------------------- *
 * OrcaRouter credential seam and provider client.
 *
 * The application's runtime path (Tauri IPC -> `omniget_core::core::orcarouter`)
 * owns the desktop flow. What lives here is the same contract expressed where
 * the repository can execute and test it without a Tauri host: the CLI, the
 * evidence harness and the unit tests all drive these functions, and the
 * SvelteKit settings page derives its options from `filterFor` so the list the
 * user sees comes from one predicate rather than a second copy of the rules.
 *
 * Two entrances, one credential. `apiKeyAdapter` (a pasted `sk-orca-…` key) and
 * `pkceAdapter` (OAuth 2.0 + PKCE, S256) are adapters over the same
 * `CredentialAdapter` interface and both produce the same `CredentialResult`, so
 * `createOrcarouterClient` cannot tell which one supplied the key.
 * ------------------------------------------------------------------------- */

/** The public origin that serves the consent screen and the code exchange. */
export const ORCA_AUTH_ORIGIN = "https://www.orcarouter.ai";
/** The public origin that serves inference and the model directory. */
export const ORCA_API_BASE = "https://api.orcarouter.ai/v1";
/** Fixed authorize path. Never derived by appending `/v1` to a hostname. */
export const ORCA_AUTHORIZE_PATH = "/auth";
/** Fixed exchange path, on the auth origin. `{api}/v1/auth/keys` is a 404. */
export const ORCA_EXCHANGE_PATH = "/api/v1/auth/keys";
export const ORCA_APP_NAME = "OmniGet";
/** Scope this client asks for; the exchange response is the granted truth. */
export const ORCA_REQUIRED_SCOPE = "api";
export const ORCA_AUTH_TIMEOUT_MS = 300_000;

export type Origins = { auth: string; api: string };

/**
 * A loopback host, the only place `http://` is allowed. A remote origin must be
 * HTTPS: silently accepting plaintext would put the code and the key on the wire.
 */
export function isLoopbackHost(host: string): boolean {
  const h = host.trim().toLowerCase().replace(/^\[|\]$/g, "");
  return h === "localhost" || h === "127.0.0.1" || h === "::1";
}

function normalizeOrigin(value: string, label: string): string {
  const raw = value.trim().replace(/\/+$/, "");
  let url: URL;
  try {
    url = new URL(raw);
  } catch {
    throw new Error(`${label} is not a valid URL`);
  }
  if (url.protocol !== "https:" && !(url.protocol === "http:" && isLoopbackHost(url.hostname))) {
    throw new Error(`${label} must use HTTPS (http is allowed only on loopback)`);
  }
  return raw;
}

/**
 * Resolve the two origins. Explicit configuration wins; nothing is derived from
 * the other origin, because the auth and inference services are separate hosts.
 */
export function resolveOrigins(auth?: string | null, api?: string | null): Origins {
  return {
    auth: normalizeOrigin(auth?.trim() || ORCA_AUTH_ORIGIN, "auth base"),
    api: normalizeOrigin(api?.trim() || ORCA_API_BASE, "API base"),
  };
}

export function authorizeUrl(
  origins: Origins,
  params: { challenge: string; state: string; callbackUrl: string; appName?: string; scope?: string },
): string {
  const url = new URL(ORCA_AUTHORIZE_PATH, origins.auth + "/");
  url.searchParams.set("callback_url", params.callbackUrl);
  url.searchParams.set("code_challenge", params.challenge);
  url.searchParams.set("code_challenge_method", "S256");
  url.searchParams.set("state", params.state);
  url.searchParams.set("app_name", params.appName ?? ORCA_APP_NAME);
  url.searchParams.set("scope", params.scope ?? ORCA_REQUIRED_SCOPE);
  return url.toString();
}

export function exchangeUrl(origins: Origins): string {
  return origins.auth + ORCA_EXCHANGE_PATH;
}

/* ----------------------------- PKCE primitives ---------------------------- */

/** base64url without padding, the only encoding the challenge accepts. */
export function base64Url(bytes: Uint8Array): string {
  let binary = "";
  for (const b of bytes) binary += String.fromCharCode(b);
  const b64 =
    typeof btoa === "function" ? btoa(binary) : Buffer.from(bytes).toString("base64");
  return b64.replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

type RandomSource = (length: number) => Uint8Array;
type Digest = (data: Uint8Array) => Promise<Uint8Array>;

/** Cryptographically random bytes from the platform CSPRNG, never a PRNG. */
export const cryptoRandom: RandomSource = (length) => {
  const out = new Uint8Array(length);
  globalThis.crypto.getRandomValues(out);
  return out;
};

const sha256: Digest = async (data) => {
  const digest = await globalThis.crypto.subtle.digest("SHA-256", data as BufferSource);
  return new Uint8Array(digest);
};

export type PkceAttempt = {
  /** Monotonic id; a response from a superseded attempt must not write state. */
  attempt: number;
  authorizeUrl: string;
  callbackUrl: string;
  state: string;
  /** Stays in this process until the exchange. Never in a URL, log or error. */
  verifier: string;
  challenge: string;
  startedAtMs: number;
};

/**
 * Begin a sign-in: fresh verifier and state from the CSPRNG on every attempt,
 * challenge = base64url(sha256(verifier)), S256 unconditionally.
 */
export async function beginPkce(
  deps: { origins: Origins; callbackUrl: string; attempt: number; nowMs: number; appName?: string },
  random: RandomSource = cryptoRandom,
  digest: Digest = sha256,
): Promise<PkceAttempt> {
  const verifier = base64Url(random(32));
  const state = base64Url(random(16));
  const challenge = base64Url(await digest(new TextEncoder().encode(verifier)));
  return {
    attempt: deps.attempt,
    authorizeUrl: authorizeUrl(deps.origins, {
      challenge,
      state,
      callbackUrl: deps.callbackUrl,
      appName: deps.appName,
    }),
    callbackUrl: deps.callbackUrl,
    state,
    verifier,
    challenge,
    startedAtMs: deps.nowMs,
  };
}

/** Length-independent, value-constant-time comparison for the CSRF `state`. */
export function constantTimeEqual(a: string, b: string): boolean {
  const left = new TextEncoder().encode(a);
  const right = new TextEncoder().encode(b);
  let diff = left.length ^ right.length;
  const n = Math.max(left.length, right.length);
  for (let i = 0; i < n; i++) diff |= (left[i] ?? 0) ^ (right[i] ?? 0);
  return diff === 0;
}

/* --------------------------- credential interface ------------------------- */

export type CredentialMethod = "api_key" | "pkce";

/** What every adapter hands downstream. The source is metadata, never a branch. */
export type CredentialResult = {
  key: string;
  method: CredentialMethod;
  account: string | null;
  scope: string | null;
  generation: number;
};

export type StoredCredential = CredentialResult & { needs_reauth?: boolean };

/** Persistence seam. The host supplies the real store (600 file, keyring, …). */
export interface CredentialVault {
  read(): Promise<StoredCredential | null> | StoredCredential | null;
  write(credential: StoredCredential): Promise<void> | void;
  clear(): Promise<void> | void;
}

/** In-memory vault for tests and for hosts that persist elsewhere. */
export function memoryVault(initial: StoredCredential | null = null): CredentialVault {
  let current = initial;
  return {
    read: () => current,
    write: (credential) => {
      current = credential;
    },
    clear: () => {
      current = null;
    },
  };
}

/**
 * One interface, two adapters. Nothing downstream may ask which one produced a
 * credential; the provider request path only ever sees `acquire()`.
 */
export interface CredentialAdapter {
  readonly method: CredentialMethod;
  acquire(input?: unknown): Promise<CredentialResult>;
}

/** Mask a key for display: never the whole secret, never nothing. */
export function maskKey(key: string): string {
  const trimmed = key.trim();
  if (trimmed.length <= 4) return "••••";
  return "••••" + trimmed.slice(-4);
}

/** A malformed key is refused without echoing it back into the message. */
export function assertApiKeyShape(key: string): string {
  const trimmed = key.trim();
  if (!/^sk-orca-[A-Za-z0-9_-]{8,}$/.test(trimmed)) {
    throw new Error("That does not look like an OrcaRouter API key (sk-orca-…).");
  }
  return trimmed;
}

/**
 * Adapter 1 — a key the user already has. It only validates shape and shape
 * only; the key is never logged, never returned in an error and never re-exported
 * to the browser beyond its masked tail.
 */
export function apiKeyAdapter(
  vault: CredentialVault,
  input: { key: string; generation: number; account?: string | null; scope?: string | null },
): CredentialAdapter {
  return {
    method: "api_key",
    async acquire() {
      const key = assertApiKeyShape(input.key);
      const credential: StoredCredential = {
        key,
        method: "api_key",
        account: input.account ?? null,
        scope: input.scope ?? ORCA_REQUIRED_SCOPE,
        generation: input.generation,
      };
      await vault.write(credential);
      return credential;
    },
  };
}

export type PkceExchangeResponse = { key: string; user_id?: string; scope?: string };
export type PkceErrorKind =
  | "denied"
  | "state_mismatch"
  | "expired_or_reused_code"
  | "challenge_downgrade"
  | "rate_limited"
  | "network"
  | "protocol";

/** A refusal that is safe to show: no code, verifier or key in the text. */
export class PkceError extends Error {
  readonly kind: PkceErrorKind;
  constructor(kind: PkceErrorKind, message: string) {
    super(message);
    this.name = "PkceError";
    this.kind = kind;
  }
}

/**
 * Adapter 2 — OAuth 2.0 + PKCE (Flow A loopback, or Flow B when the host hands
 * over a pasted code). The host owns how the code arrives; this owns the
 * protocol: state first, then the exchange on the auth origin, then a granted
 * scope check, then persistence of the returned API key.
 */
export function pkceAdapter(deps: {
  vault: CredentialVault;
  origins: Origins;
  fetchImpl: FetchLike;
  nowMs?: number;
  /** Refuses a challenge method other than S256, per the consent contract. */
}): CredentialAdapter & {
  begin(attempt: number, callbackUrl: string): Promise<PkceAttempt>;
  complete(
    pending: PkceAttempt,
    callback: { code?: string | null; state?: string | null; error?: string | null },
  ): Promise<CredentialResult>;
} {
  let current: PkceAttempt | null = null;

  return {
    method: "pkce",
    async begin(attempt: number, callbackUrl: string) {
      current = await beginPkce({
        origins: deps.origins,
        callbackUrl,
        attempt,
        nowMs: deps.nowMs ?? Date.now(),
      });
      return current;
    },
    async complete(pending, callback) {
      // A superseded attempt may not install anything, even if its state matches
      // — its own callback arriving late must not overwrite a newer sign-in.
      if (!current || pending.attempt !== current.attempt) {
        throw new PkceError("protocol", "That sign-in attempt was superseded; start again.");
      }
      // The state check happens before the code is looked at, let alone sent.
      if (!callback.state || !constantTimeEqual(callback.state, pending.state)) {
        throw new PkceError("state_mismatch", "The sign-in response did not match this attempt.");
      }
      if (callback.error) {
        throw new PkceError(
          callback.error === "access_denied" ? "denied" : "protocol",
          callback.error === "access_denied"
            ? "Sign-in was declined. No credential was stored."
            : "The sign-in service refused the request.",
        );
      }
      if (!callback.code) {
        throw new PkceError("protocol", "The sign-in response carried no code.");
      }
      const granted = await exchangeCode(deps, pending, callback.code);
      const credential: StoredCredential = {
        key: granted.key,
        method: "pkce",
        account: granted.account,
        scope: granted.scope,
        generation: pending.attempt,
      };
      // The previous secret is only replaced once a new one is in hand.
      await deps.vault.write(credential);
      return credential;
    },
    acquire: async () => {
      if (!current) throw new PkceError("protocol", "No sign-in attempt is in flight.");
      throw new PkceError("protocol", "complete the sign-in with the returned code");
    },
  };
}

async function exchangeCode(
  deps: { origins: Origins; fetchImpl: FetchLike },
  pending: PkceAttempt,
  code: string,
): Promise<{ key: string; account: string | null; scope: string }> {
  const body = {
    code,
    code_verifier: pending.verifier,
    code_challenge_method: "S256",
  };
  let response: FetchResponse;
  try {
    response = await deps.fetchImpl(exchangeUrl(deps.origins), {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
    });
  } catch {
    throw new PkceError("network", "Could not reach the sign-in service.");
  }
  if (response.status === 429) {
    throw new PkceError("rate_limited", "Too many sign-in attempts. Try again shortly.");
  }
  if (response.status === 400) {
    throw new PkceError("challenge_downgrade", "The sign-in service refused the PKCE challenge.");
  }
  if (response.status === 403) {
    throw new PkceError(
      "expired_or_reused_code",
      "That sign-in code is expired or already used. Start again.",
    );
  }
  if (!response.ok) {
    throw new PkceError("protocol", `The sign-in service answered ${response.status}.`);
  }
  let payload: PkceExchangeResponse;
  try {
    payload = (await response.json()) as PkceExchangeResponse;
  } catch {
    throw new PkceError("protocol", "The sign-in service returned an unreadable response.");
  }
  if (typeof payload.key !== "string" || payload.key.trim() === "") {
    throw new PkceError("protocol", "The sign-in service returned no key.");
  }
  // What was granted, not what was asked for.
  const scope = typeof payload.scope === "string" ? payload.scope : "";
  if (scope !== ORCA_REQUIRED_SCOPE) {
    throw new PkceError(
      "protocol",
      `This app needs the "${ORCA_REQUIRED_SCOPE}" scope; the grant came back as "${scope || "none"}".`,
    );
  }
  return { key: payload.key, account: payload.user_id ?? null, scope };
}

/* ------------------------------ provider client --------------------------- */

export type FetchLike = (url: string, init?: Record<string, unknown>) => Promise<FetchResponse>;
export type FetchResponse = {
  ok: boolean;
  status: number;
  json(): Promise<unknown>;
};

export type ChatMessage = { role: "system" | "user" | "assistant"; content: string };

export type ClientError = {
  kind: "needs_reauth" | "forbidden" | "rate_limited" | "network" | "http" | "protocol";
  status: number | null;
  message: string;
};

export function isClientError(value: unknown): value is ClientError {
  return !!value && typeof value === "object" && "kind" in (value as Record<string, unknown>);
}

/**
 * Read the `error.code` from a refusal body, without surfacing arbitrary server
 * text or anything credential-shaped to the caller.
 */
async function readDenialCode(response: FetchResponse): Promise<string | null> {
  try {
    const body = (await response.json()) as { error?: { code?: unknown } };
    const code = body?.error?.code;
    return typeof code === "string" ? code : null;
  } catch {
    return null;
  }
}

/**
 * The provider client. It takes a credential *source*, not a key and not an
 * adapter: `apiKeyAdapter` and `pkceAdapter` are interchangeable here, which is
 * what "the downstream does not care where the credential came from" means in
 * practice.
 */
export function createOrcarouterClient(deps: {
  credential: { resolve(): Promise<CredentialResult> };
  origins: Origins;
  fetchImpl: FetchLike;
}) {
  const auth = async () => deps.credential.resolve();
  const headers = (credential: CredentialResult) => ({
    Authorization: `Bearer ${credential.key}`,
    "Content-Type": "application/json",
  });

  async function send(path: string, init: Record<string, unknown>): Promise<unknown> {
    const credential = await auth();
    let response: FetchResponse;
    try {
      response = await deps.fetchImpl(deps.origins.api + path, {
        ...init,
        headers: { ...headers(credential), ...((init.headers as object) ?? {}) },
      });
    } catch {
      throw {
        kind: "network",
        status: null,
        message: "Could not reach OrcaRouter.",
      } satisfies ClientError;
    }
    if (response.status === 401) {
      // The credential itself was refused. Terminal: never refreshed, never
      // retried, and the caller marks exactly this generation for re-auth.
      throw {
        kind: "needs_reauth",
        status: 401,
        message: "OrcaRouter refused this credential. Sign in again.",
      } satisfies ClientError;
    }
    if (response.status === 403) {
      // The credential is valid but is not permitted this route or model — a
      // different problem with a different fix, so it gets its own kind rather
      // than being reported as a broken key.
      const denial = await readDenialCode(response);
      throw {
        kind: "forbidden",
        status: 403,
        message:
          denial === "model_access_denied"
            ? "This OrcaRouter key is not allowed to use that model. Enable it for the key in the OrcaRouter console."
            : "OrcaRouter refused this request for this credential. Check the key's permissions.",
      } satisfies ClientError;
    }
    if (response.status === 429) {
      throw {
        kind: "rate_limited",
        status: 429,
        message: "OrcaRouter is rate limiting this key. Try again shortly.",
      } satisfies ClientError;
    }
    if (!response.ok) {
      throw {
        kind: "http",
        status: response.status,
        message: `OrcaRouter answered ${response.status}.`,
      } satisfies ClientError;
    }
    return response.json();
  }

  return {
    /** The capability-filtered directory, straight from `/v1/models`. */
    async listModels(capability: string, modality?: string | null): Promise<ModelOption[]> {
      const params = new URLSearchParams({ capability });
      if (modality) params.set("modality", modality);
      const payload = await send(`/models?${params.toString()}`, { method: "GET" });
      return parseCatalog(payload).map(toOption);
    },
    async chat(model: string, messages: ChatMessage[]): Promise<{ content: string; model: string }> {
      const payload = (await send("/chat/completions", {
        method: "POST",
        body: JSON.stringify({ model, messages, stream: false }),
      })) as { model?: string; choices?: { message?: { content?: string } }[] };
      const content = payload?.choices?.[0]?.message?.content;
      if (typeof content !== "string") {
        throw {
          kind: "protocol",
          status: null,
          message: "OrcaRouter returned no completion.",
        } satisfies ClientError;
      }
      return { content, model: payload.model ?? model };
    },
  };
}

/** A credential source that reads once from a vault and caches the result. */
export function vaultCredentialSource(vault: CredentialVault) {
  let cached: CredentialResult | null = null;
  return {
    async resolve(): Promise<CredentialResult> {
      if (cached) return cached;
      const stored = await vault.read();
      if (!stored || !stored.key) throw new Error("No OrcaRouter credential is stored.");
      cached = stored;
      return stored;
    },
    invalidate() {
      cached = null;
    },
  };
}

/* ---------------------------- catalog + filtering ------------------------- */

/** Endpoint routes that prove a model is text chat, per the directory. */
export const TEXT_ENDPOINT_TYPES = ["openai", "anthropic", "gemini", "openai-response"];
/**
 * Routes that are unambiguously *not* a text chat model. A model that carries
 * one of these is excluded from the chat list even if it also speaks a text
 * route, because that combination means a dedicated non-chat model.
 */
export const NON_CHAT_ENDPOINT_TYPES = [
  "image-generation",
  "openai-video",
  "jina-rerank",
  "embedding",
  "embeddings",
];

export type CatalogEntrance =
  | "chat"
  | "multimodal"
  | "embedding"
  | "image"
  | "video"
  | "rerank";

function endpointsOf(model: RawModel): string[] {
  return (model.supported_endpoint_types ?? model.endpoint_types ?? []).map((e) =>
    String(e).toLowerCase(),
  );
}

/**
 * A model with no route metadata at all cannot be checked here, and the rule is
 * that an unverifiable model is not offered: the selector fails closed and shows
 * its empty state rather than a list nothing vouches for.
 */
function declaresRoutes(model: RawModel): boolean {
  return endpointsOf(model).length > 0;
}

function modalitiesOf(model: RawModel): string[] {
  // The live directory nests modalities under `architecture`; the backend's
  // reduced view flattens them. Both carry the same declaration, so read either.
  const declared = model.architecture?.input_modalities ?? model.input_modalities ?? [];
  return declared.map((m) => String(m).toLowerCase());
}

export type RawModel = {
  id: string;
  name?: string;
  context_length?: number | null;
  max_completion_tokens?: number | null;
  supported_endpoint_types?: string[];
  endpoint_types?: string[];
  input_modalities?: string[];
  architecture?: { input_modalities?: string[]; output_modalities?: string[] };
  reasoning?: boolean;
  reasoning_efforts?: string[];
};

/**
 * Whether a directory entry may be offered for one entrance.
 *
 * The rules come from the endpoint and modality metadata only — never from the
 * model's name. A missing `architecture.input_modalities` fails closed for a
 * multimodal entrance: "not declared" is not "declared compatible".
 */
export function satisfies(model: RawModel, entrance: CatalogEntrance, modality?: string | null): boolean {
  const types = endpointsOf(model);
  if (!declaresRoutes(model)) return false;
  const isChat =
    TEXT_ENDPOINT_TYPES.some((t) => types.includes(t)) &&
    !NON_CHAT_ENDPOINT_TYPES.some((t) => types.includes(t));
  switch (entrance) {
    case "chat":
      return isChat;
    case "multimodal":
      if (!isChat) return false;
      return !!modality && modalitiesOf(model).includes(modality.toLowerCase());
    case "embedding":
      return types.includes("embeddings") || types.includes("embedding");
    case "image":
      return types.includes("image-generation");
    case "video":
      return types.includes("openai-video");
    case "rerank":
      return types.includes("jina-rerank");
    default:
      return false;
  }
}

/** Project one directory entry onto the option the selector renders. */
export function toOption(model: RawModel): ModelOption {
  return {
    id: model.id,
    name: model.name || model.id,
    context_length: model.context_length ?? null,
    max_completion_tokens: model.max_completion_tokens ?? null,
    input_modalities: modalitiesOf(model),
    // Carried through, not dropped: the selector re-applies the capability
    // predicate to the options it holds, so they must keep the route metadata
    // that predicate reads.
    endpoint_types: endpointsOf(model),
    reasoning: model.reasoning === true,
    reasoning_efforts: model.reasoning_efforts ?? [],
  };
}

/** Filter a directory for one entrance, preserving the directory's own order. */
export function filterFor(
  models: RawModel[],
  entrance: CatalogEntrance,
  modality?: string | null,
): ModelOption[] {
  return models.filter((m) => satisfies(m, entrance, modality)).map(toOption);
}

/**
 * Read the directory response body.
 *
 * The directory answers `{ data: [...] }`. A body that is not a list of entries
 * is a protocol failure, not an empty workspace: reporting "no models" for a
 * broken response would hide a real outage behind a plausible-looking state.
 */
export function parseCatalog(payload: unknown): RawModel[] {
  const body = payload as { data?: unknown; models?: unknown } | null;
  const list = Array.isArray(payload)
    ? payload
    : Array.isArray(body?.data)
      ? body!.data
      : Array.isArray(body?.models)
        ? body!.models
        : null;
  if (!list) throw new Error("The OrcaRouter directory returned an unexpected body.");
  return list.filter((m): m is RawModel => !!m && typeof (m as RawModel).id === "string");
}

/* --------------------------- credential lifecycle ------------------------- */

/**
 * The stored credential plus the one piece of volatile state that must not leak
 * across sign-ins: which generation was rejected.
 *
 * A 401/403 marks exactly the generation whose request was refused. A later
 * sign-in installs a new generation and starts unmarked, so a stale rejection
 * can never make a freshly connected credential look revoked. The rejected
 * secret is kept until a new one replaces it — dropping it silently would sign
 * the user out of a workspace they can still reach with a fresh key.
 */
export class CredentialState {
  private credential: StoredCredential | null = null;
  private rejectedGeneration: number | null = null;

  constructor(initial: StoredCredential | null = null) {
    if (initial) this.accept(initial);
  }

  /** Install a credential from either adapter, and clear the rejection flag. */
  accept(credential: StoredCredential): void {
    this.credential = credential;
    this.rejectedGeneration = null;
  }

  /** Record that `generation`'s request was refused by the relay. */
  reject(generation: number): boolean {
    if (!this.credential || this.credential.generation !== generation) return false;
    this.rejectedGeneration = generation;
    return true;
  }

  get needsReauth(): boolean {
    return this.rejectedGeneration !== null && this.rejectedGeneration === this.credential?.generation;
  }

  get current(): StoredCredential | null {
    return this.credential;
  }

  clear(): void {
    this.credential = null;
    this.rejectedGeneration = null;
  }
}

/**
 * The options the selector may render for one entrance.
 *
 * The backend already filters (`omniget_core::core::orcarouter`); this re-applies
 * the same predicate to what actually came back, so the `<option>` list the user
 * sees is derived from the directory response rather than trusted wholesale. It
 * is also what makes the rule testable from the frontend: change the entrance or
 * attach a file, and the options change with it.
 */
export function optionsForEntrance(
  view: CatalogView | null,
  entrance: CatalogEntrance,
  modality?: string | null,
): ModelOption[] {
  if (!view) return [];
  return filterFor(view.models as RawModel[], entrance, modality);
}

/**
 * Reconcile a stored selection against the options for an entrance.
 *
 * Returns the id to keep, or `null` when the entrance can no longer use it — the
 * caller clears the field and asks the user to choose again.
 */
export function reconcileForEntrance(
  view: CatalogView | null,
  selected: string | null | undefined,
  entrance: CatalogEntrance,
  modality?: string | null,
): string | null {
  return reconcileSelection(optionsForEntrance(view, entrance, modality), selected);
}
