import { describe, expect, it } from "vitest";
import { agentChoices, endpointNeedsAttention, isBuiltinAgent, modelForKind, modelHint, modelNeedsAttention, visibleEnvKinds } from "./ai-settings";

describe("modelForKind", () => {
  it("prefills the provider's model hint when switching", () => {
    expect(modelForKind("anthropic", { kind: "openai", model: "gpt-4o" })).toBe(modelHint("anthropic"));
    expect(modelForKind("openai", null)).toBe("gpt-4o-mini");
  });

  it("restores the saved model when going back to the saved provider", () => {
    expect(modelForKind("openai", { kind: "openai", model: "gpt-4.1" })).toBe("gpt-4.1");
  });

  it("falls back to the hint when the saved model is empty", () => {
    expect(modelForKind("ollama", { kind: "ollama", model: " " })).toBe("llama3.2");
  });

  it("leaves the model empty for providers without a hint", () => {
    expect(modelForKind("custom", null)).toBe("");
  });
});

describe("modelNeedsAttention", () => {
  it("is false when the app AI is off", () => {
    expect(modelNeedsAttention("none", "")).toBe(false);
  });

  it("is true when the model is empty", () => {
    expect(modelNeedsAttention("openai", "")).toBe(true);
  });

  it("is true for a provider with no known model", () => {
    expect(modelNeedsAttention("custom", "my-model")).toBe(true);
  });

  it("is false for a known provider with a model", () => {
    expect(modelNeedsAttention("anthropic", "claude-sonnet-4-5")).toBe(false);
  });
});

describe("visibleEnvKinds", () => {
  it("hides WSL outside Windows", () => {
    expect(visibleEnvKinds(false, { wsl: false })).not.toContain("wsl");
  });

  it("shows WSL on Windows", () => {
    expect(visibleEnvKinds(true, null)).toContain("wsl");
  });

  it("keeps WSL visible when it is already on, so it can be turned off", () => {
    expect(visibleEnvKinds(false, { wsl: true })).toContain("wsl");
  });
});

describe("agentChoices", () => {
  it("lists the built-in agents, then the presets", () => {
    expect(agentChoices(["aider"], "shell")).toEqual(["shell", "claude", "codex", "gemini", "opencode", "ollama", "aider"]);
  });

  it("drops presets named like a built-in agent", () => {
    const list = agentChoices(["claude", "aider", "aider"], "claude");
    expect(list).toEqual(["shell", "claude", "codex", "gemini", "opencode", "ollama", "aider"]);
    expect(new Set(list).size).toBe(list.length);
  });

  it("keeps an unknown current value selectable", () => {
    expect(agentChoices([], "gone")).toContain("gone");
  });
});

describe("endpointNeedsAttention", () => {
  it("shows the endpoint for a custom endpoint", () => {
    expect(endpointNeedsAttention("custom", true)).toBe(true);
  });

  it("shows the endpoint for a relay that needs the user's own host", () => {
    expect(endpointNeedsAttention("newapi", true)).toBe(true);
  });

  it("keeps the Ollama default in advanced", () => {
    expect(endpointNeedsAttention("ollama", true)).toBe(false);
  });

  it("is false for fixed hosts and when AI is off", () => {
    expect(endpointNeedsAttention("openai", false)).toBe(false);
    expect(endpointNeedsAttention("none", true)).toBe(false);
  });
});

describe("isBuiltinAgent", () => {
  it("matches built-in agent names", () => {
    expect(isBuiltinAgent("codex")).toBe(true);
    expect(isBuiltinAgent(" shell ")).toBe(true);
    expect(isBuiltinAgent("aider")).toBe(false);
  });
});
