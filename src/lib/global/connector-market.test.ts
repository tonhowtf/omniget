import { describe, expect, it } from "vitest";
import { authProblem, errorKey, looksLikeUrl } from "./connector-market";

describe("errorKey", () => {
  it("names the failed check", () => {
    expect(errorKey("signature: the catalog signature is not valid")).toBe("global_dl.market.err.signature");
    expect(errorKey("hash: expected a, got b")).toBe("global_dl.market.err.hash");
    expect(errorKey("mismatch: the package says")).toBe("global_dl.market.err.mismatch");
    expect(errorKey("network: https://x: HTTP 404")).toBe("global_dl.market.err.network");
  });

  it("keeps the existing message for sites that opted out", () => {
    expect(errorKey("The owner of this site asked OmniGet not to support it.")).toBe("errors.platform_opted_out");
  });

  it("falls back for anything else", () => {
    expect(errorKey("disk full")).toBe("global_dl.market.err.generic");
  });
});

describe("authProblem", () => {
  it("recognises connector messages that ask to sign in", () => {
    expect(authProblem("resolve: Error: Not signed in to Hotmart. Sign in at consumer.hotmart.com")).toBe(true);
    expect(authProblem("HTTP 403 for https://www.udemy.com/api-2.0/x")).toBe(true);
  });

  it("leaves other failures alone", () => {
    expect(authProblem("HTTP 500 for https://x")).toBe(false);
    expect(authProblem("connection reset")).toBe(false);
  });
});

describe("looksLikeUrl", () => {
  it("accepts links and bare hosts", () => {
    expect(looksLikeUrl("https://hotmart.com/pt-br/club/x")).toBe(true);
    expect(looksLikeUrl("consumer.hotmart.com")).toBe(true);
  });

  it("refuses search text", () => {
    expect(looksLikeUrl("how to cook rice")).toBe(false);
    expect(looksLikeUrl("rice")).toBe(false);
    expect(looksLikeUrl("")).toBe(false);
  });
});
