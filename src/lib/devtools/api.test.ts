import { describe, expect, it } from "vitest";
import { parseError } from "./api";

describe("parseError", () => {
  it("reads the code and the recovery fields of a devtools error", () => {
    const raw =
      'NO_MATCH: {"candidates":[".product-price"],"message":"no match for selector .price","next_step":"retry with one of the candidates","retryable":false}';
    expect(parseError(raw)).toEqual({
      code: "NO_MATCH",
      message: "no match for selector .price",
      retryable: false,
      nextStep: "retry with one of the candidates",
      candidates: [".product-price"],
    });
  });

  it("keeps the code of an error whose detail is plain text", () => {
    expect(parseError("TOOL_NOT_AUTHORIZED: dev_ports op=kill needs the devtools_write scope")).toEqual({
      code: "TOOL_NOT_AUTHORIZED",
      message: "dev_ports op=kill needs the devtools_write scope",
      retryable: false,
      nextStep: "",
      candidates: [],
    });
  });

  it("falls back to a generic code for anything else", () => {
    expect(parseError("task panicked").code).toBe("OPERATION_FAILED");
    expect(parseError(new Error("boom")).message).toBe("Error: boom");
  });
});
