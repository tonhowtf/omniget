import { describe, expect, it, vi } from "vitest";
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));
import { diagnoseConnectionError, sanitizeError } from "./connection-setup";

describe("diagnoseConnectionError", () => {
  it("names the concrete problem with a way out", () => {
    expect(diagnoseConnectionError("ERR_CLI_NOT_FOUND: claude not found on PATH", "subscription").problem).toBe("missing_executable");
    expect(diagnoseConnectionError("ERR_CLI_AUTH: not logged in", "subscription")).toMatchObject({ problem: "login_expired", actions: ["sign_in", "retry", "switch"] });
    expect(diagnoseConnectionError("ERR_LLM_AUTH: 401 invalid api key", "api").problem).toBe("credential_invalid");
    expect(diagnoseConnectionError("ERR_LLM_MODEL: model qwen9 not found", "local").problem).toBe("model_unavailable");
    expect(diagnoseConnectionError("ERR_LLM_NET: connection refused (127.0.0.1:11434)", "local").problem).toBe("local_service_down");
    expect(diagnoseConnectionError("ERR_LLM_NET: timed out", "api").problem).toBe("network");
    expect(diagnoseConnectionError("Test incomplete: length", "api").problem).toBe("no_answer");
    expect(diagnoseConnectionError("weird", "api")).toMatchObject({ problem: "unknown", titleKey: "assist.bots.connection.problem.unknown" });
  });

  it("never shows a secret in the detail", () => {
    const text = "ERR_LLM_AUTH: bad key sk-proj-abcdefghijklmnopqrstuvwx with Bearer eyJhbGciOiJIUzI1NiJ9.payload and ?api_key=zzz123";
    const out = sanitizeError(text);
    expect(out).not.toContain("sk-proj-abcdefghijklmnopqrstuvwx");
    expect(out).not.toContain("eyJhbGciOiJIUzI1NiJ9");
    expect(out).not.toContain("zzz123");
    expect(diagnoseConnectionError(text, "api").detail).toBe(out);
  });
});
