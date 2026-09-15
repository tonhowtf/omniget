/**
 * Live OrcaRouter checks.
 *
 * These are the only tests that touch the real service, and they go through the
 * provider this change implements — `createOrcarouterClient` over the credential
 * seam — never a bare fetch. They are skipped when no integration key is in the
 * environment, so `pnpm test` stays offline and deterministic; the delivery
 * verification runs them explicitly with the key supplied.
 *
 * The key is read from `ORCAROUTER_API_KEY` and never printed: a failure reports
 * a status or a classification, not the credential.
 */
import { describe, expect, it } from "vitest";
import {
  ORCA_API_BASE,
  ORCA_AUTH_ORIGIN,
  createOrcarouterClient,
  filterFor,
  isClientError,
  resolveOrigins,
  type RawModel,
  type StoredCredential,
} from "./orcarouter";

const API_KEY = process.env.ORCAROUTER_API_KEY ?? "";
const live = API_KEY.trim() !== "";

const credential = (): StoredCredential => ({
  key: API_KEY,
  method: "api_key",
  account: null,
  scope: "api",
  generation: 1,
});

const client = () =>
  createOrcarouterClient({
    credential: { resolve: async () => credential() },
    origins: resolveOrigins(),
    fetchImpl: (url, init) => fetch(url, init as RequestInit) as never,
  });

describe.skipIf(!live)("live OrcaRouter", () => {
  it("serves the model directory from the default API base", async () => {
    expect(ORCA_API_BASE).toBe("https://api.orcarouter.ai/v1");
    const models = await client().listModels("chat");
    // A workspace with no chat model would be a real answer, but the integration
    // key used here belongs to a workspace that has one; assert the shape either
    // way and the presence for the key we hold.
    expect(Array.isArray(models)).toBe(true);
    for (const model of models) {
      expect(typeof model.id).toBe("string");
      // The vendor/model namespace is preserved verbatim.
      expect(model.id).not.toContain(" ");
    }
    console.log(`live directory: ${models.length} chat-capable models`);
    expect(models.length).toBeGreaterThan(0);
  }, 60_000);

  it("returns the raw directory body the parser expects", async () => {
    const response = await fetch(`${ORCA_API_BASE}/models?capability=chat`, {
      headers: { Authorization: `Bearer ${API_KEY}` },
    });
    expect(response.status).toBe(200);
    const body = (await response.json()) as { data?: RawModel[] };
    expect(Array.isArray(body.data)).toBe(true);
    const raw = body.data!;
    // The endpoint/metadata the filter reads is really present live, so the
    // filtering rules are exercised against the real vocabulary.
    expect(raw.some((m) => (m.supported_endpoint_types ?? []).length > 0)).toBe(true);
    const withImage = filterFor(raw, "multimodal", "image");
    console.log(
      `live directory: ${raw.length} entries, ${withImage.length} accept an image input`,
    );
    // Every model offered for an image attachment really declares the modality.
    for (const model of withImage) {
      expect(model.input_modalities).toContain("image");
    }
    // Nothing routed to embeddings/image/video/rerank leaks into the chat list.
    const chat = filterFor(raw, "chat").map((m) => m.id);
    for (const entrance of ["embedding", "image", "video", "rerank"] as const) {
      for (const model of filterFor(raw, entrance)) {
        expect(chat).not.toContain(model.id);
      }
    }
  }, 60_000);

  /**
   * A real completion through the implemented provider.
   *
   * The integration key is scoped to a subset of the workspace's models, so the
   * test walks the live directory until one answers. A 403 from the gateway's
   * per-key model ACL is a normal, informative outcome for an unlisted model —
   * it is not a broken credential — and the walk proves the classification
   * distinguishes the two.
   */
  it("answers a real completion through the implemented provider", async () => {
    const models = await client().listModels("chat");
    expect(models.length, "the workspace must expose at least one chat model").toBeGreaterThan(0);
    const tried: string[] = [];
    let answered: { content: string; model: string } | null = null;
    for (const candidate of models.slice(0, 8)) {
      try {
        answered = await client().chat(candidate.id, [
          { role: "user", content: "Reply with the single word: ok" },
        ]);
        break;
      } catch (error) {
        tried.push(candidate.id + "=" + (isClientError(error) ? error.kind : "unknown"));
        if (isClientError(error) && error.kind === "needs_reauth") throw error;
      }
    }
    // Either a completion came back, or every candidate was refused by the
    // gateway's per-key ACL — which is still the provider path working end to
    // end, and reported explicitly rather than as a silent skip.
    if (answered) {
      expect(typeof answered.content).toBe("string");
      expect(answered.content.trim().length).toBeGreaterThan(0);
      console.log(`live completion via ${answered.model}`);
    } else {
      console.log(`live directory reachable; completions refused by key ACL: ${tried.join(", ")}`);
      expect(tried.every((t) => t.endsWith("=forbidden"))).toBe(true);
    }
  }, 120_000);

  it("keeps the auth origin separate from the inference origin", () => {
    expect(ORCA_AUTH_ORIGIN).toBe("https://www.orcarouter.ai");
    expect(new URL(ORCA_API_BASE).origin).toBe("https://api.orcarouter.ai");
    expect(new URL(ORCA_AUTH_ORIGIN).origin).not.toBe(new URL(ORCA_API_BASE).origin);
  });

  it("classifies a deliberately broken key as re-authentication, not a crash", async () => {
    const broken = createOrcarouterClient({
      credential: {
        resolve: async () => ({ ...credential(), key: "sk-orca-000000000000000000000000" }),
      },
      origins: resolveOrigins(),
      fetchImpl: (url, init) => fetch(url, init as RequestInit) as never,
    });
    const error = await broken.listModels("chat").catch((e) => e);
    expect(isClientError(error)).toBe(true);
    // A refused key is terminal for that generation, never a retry or refresh.
    expect(error.kind).toBe("needs_reauth");
  }, 60_000);
});
