import { describe, expect, it } from "vitest";
import {
  ENTRANCE_ATTACHMENTS,
  capabilityFor,
  catalogStatus,
  isSelectable,
  optionsForEntrance,
  reconcileForEntrance,
  reconcileSelection,
  toOption,
  type CatalogView,
  type ModelOption,
} from "./orcarouter";

const model = (id: string, input: string[] = ["text"]): ModelOption => ({
  id,
  name: id,
  context_length: null,
  max_completion_tokens: null,
  input_modalities: input,
  reasoning: false,
  reasoning_efforts: [],
});

const view = (models: ModelOption[], over: Partial<CatalogView> = {}): CatalogView => ({
  models,
  source: "live",
  degraded: false,
  fetched_at_ms: 1,
  live: true,
  needs_reauth: false,
  ...over,
});

describe("capabilityFor", () => {
  it("asks for plain chat when nothing non-text is attached", () => {
    expect(capabilityFor("chat", 0)).toEqual({ capability: "chat" });
  });

  it("asks for a modality-restricted chat list once media is attached", () => {
    const req = capabilityFor("chat", 1);
    expect(req.capability).toBe("multimodal");
    expect(req.modality).toBe("image");
    // Never a bare "chat" list once an attachment is in play.
    expect(req.capability).not.toBe("chat");
  });

  it("has no entrance in this repository that attaches media", () => {
    // If a media-attaching entrance is added, this assertion is the reminder
    // that ENTRANCE_ATTACHMENTS and the tests below must grow with it.
    for (const [entrance, count] of Object.entries(ENTRANCE_ATTACHMENTS)) {
      expect(count, `${entrance} must declare its real attachment count`).toBe(0);
    }
  });
});

describe("reconcileSelection", () => {
  const options = [model("orcarouter/auto"), model("deepseek/deepseek-v4.1-flash")];

  it("keeps a model that is still offered", () => {
    expect(reconcileSelection(options, "orcarouter/auto")).toBe("orcarouter/auto");
  });

  it("clears a model that the current entrance no longer offers", () => {
    // This is what makes the selection invalidate when the provider, the
    // attachment set or the task type changes.
    expect(reconcileSelection(options, "openai/gpt-5.5")).toBeNull();
  });

  it("clears a model that vanished from a refreshed catalog", () => {
    const refreshed = [model("orcarouter/auto")];
    expect(reconcileSelection(refreshed, "deepseek/deepseek-v4.1-flash")).toBeNull();
  });

  it("treats empty and missing selections as nothing selected", () => {
    expect(reconcileSelection(options, "")).toBeNull();
    expect(reconcileSelection(options, "   ")).toBeNull();
    expect(reconcileSelection(options, null)).toBeNull();
    expect(reconcileSelection(options, undefined)).toBeNull();
  });

  it("reconciles against the filtered options, not the whole catalog", () => {
    // A text-only model must not survive a multimodal list even though it
    // exists in the catalog.
    const multimodalOnly = [model("deepseek/deepseek-v4.1-flash", ["text", "image"])];
    expect(reconcileSelection(multimodalOnly, "orcarouter/auto")).toBeNull();
  });
});

describe("catalogStatus", () => {
  it("reports a successful live fetch as live", () => {
    expect(catalogStatus(view([model("a")])).kind).toBe("live");
  });

  it("labels a fallback catalog as degraded rather than live", () => {
    const seed = view([model("openai/gpt-5.5")], {
      source: "seed",
      degraded: true,
      live: false,
    });
    const status = catalogStatus(seed);
    expect(status.kind).toBe("degraded");
    expect(status.labelKey).toBe("settings.ai.orca_status_degraded");
  });

  it("labels last-known-good as degraded too", () => {
    const lkg = view([model("a")], {
      source: "last_known_good",
      degraded: true,
      live: false,
    });
    expect(catalogStatus(lkg).kind).toBe("degraded");
  });

  it("surfaces a rejected credential above the catalog state", () => {
    const stale = view([model("a")], { needs_reauth: true });
    expect(catalogStatus(stale).kind).toBe("reauth");
  });

  it("treats a missing catalog as degraded, never as live", () => {
    expect(catalogStatus(null).kind).toBe("degraded");
  });
});

describe("isSelectable", () => {
  it("is true when the catalog offers at least one model", () => {
    expect(isSelectable(view([model("a")]))).toBe(true);
  });

  it("is false for an empty catalog, so the UI shows an empty state", () => {
    // The selector must not degrade into a free-text field here.
    expect(isSelectable(view([]))).toBe(false);
    expect(isSelectable(null)).toBe(false);
  });

  it("does not treat a degraded catalog with models as unusable", () => {
    const seed = view([model("openai/gpt-5.5")], {
      source: "seed",
      degraded: true,
      live: false,
    });
    expect(isSelectable(seed)).toBe(true);
  });
});

describe("selector options", () => {
  // The route metadata the backend filtered on, as it crosses the IPC boundary.
  const chat: ModelOption = {
    ...model("deepseek/deepseek-v4-flash"),
    endpoint_types: ["openai", "openai-response"],
  };
  const vision: ModelOption = {
    ...model("deepseek/deepseek-v4.1-flash", ["text", "image"]),
    endpoint_types: ["openai", "anthropic"],
  };
  const embedder: ModelOption = {
    ...model("vendor/text-embed-3"),
    endpoint_types: ["embeddings"],
  };
  const catalog = view([chat, vision, embedder]);

  it("offers the chat-capable entries for a text entrance", () => {
    const options = optionsForEntrance(catalog, "chat");
    expect(options.map((m) => m.id)).toEqual([
      "deepseek/deepseek-v4-flash",
      "deepseek/deepseek-v4.1-flash",
    ]);
    expect(options.map((m) => m.id)).not.toContain("vendor/text-embed-3");
  });

  it("keeps only models that declare the attached modality, and fails closed", () => {
    const options = optionsForEntrance(catalog, "multimodal", "image");
    expect(options.map((m) => m.id)).toEqual(["deepseek/deepseek-v4.1-flash"]);
    // A chat model that never declared image input is not assumed to take one.
    expect(options.map((m) => m.id)).not.toContain("deepseek/deepseek-v4-flash");
  });

  it("clears a text selection once the entrance needs an image-input model", () => {
    expect(reconcileForEntrance(catalog, "deepseek/deepseek-v4-flash", "multimodal", "image")).toBeNull();
    expect(reconcileForEntrance(catalog, "deepseek/deepseek-v4.1-flash", "multimodal", "image")).toBe(
      "deepseek/deepseek-v4.1-flash",
    );
  });

  it("drops an option whose routes the view lost, rather than trusting the list", () => {
    // A view entry with no route metadata proves nothing, so it is not offered.
    const unverified: ModelOption = { ...model("vendor/opaque-1") };
    expect(optionsForEntrance(view([unverified]), "chat")).toEqual([]);
  });

  it("offers nothing from a degraded catalog that carries no models", () => {
    const empty = view([], { source: "seed", degraded: true, live: false });
    expect(optionsForEntrance(empty, "chat")).toEqual([]);
  });
});

describe("toOption", () => {
  it("carries the route metadata the selector re-filters on", () => {
    const option = toOption({
      id: "deepseek/deepseek-v4.1-flash",
      supported_endpoint_types: ["OpenAI", "Anthropic"],
      architecture: { input_modalities: ["text", "image"] },
    });
    expect(option.endpoint_types).toEqual(["openai", "anthropic"]);
    expect(option.input_modalities).toEqual(["text", "image"]);
    // Which is what keeps the option filterable after it crosses the boundary.
    expect(optionsForEntrance(view([option]), "multimodal", "image")).toHaveLength(1);
  });
});
