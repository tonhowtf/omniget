import { invoke } from "@tauri-apps/api/core";

interface ProviderModelsResponse {
  provider: string;
  models: string[];
  cached: boolean;
}

export function isLocalModelProvider(provider: string): boolean {
  return ["ollama", "lmstudio", "llama-server"].includes(provider);
}

export async function loadProviderModels(provider: string): Promise<string[]> {
  const response = await invoke<ProviderModelsResponse>("llm_models_list", { provider });
  if (!response || response.provider !== provider || !Array.isArray(response.models)
    || !response.models.every(model => typeof model === "string") || typeof response.cached !== "boolean") {
    throw new Error("ERR_LLM_MODELS_RESPONSE");
  }
  return [...new Set(response.models)];
}
