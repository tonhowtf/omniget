import { afterEach, describe, expect, it, vi } from "vitest";
import { load } from "./+page";

afterEach(() => vi.unstubAllEnvs());

describe("kitchen sink route", () => {
  it("stays reachable on the dev server", () => {
    vi.stubEnv("DEV", true);
    expect(load()).toBeUndefined();
  });

  it("redirects home outside the dev server", () => {
    vi.stubEnv("DEV", false);
    expect(() => load()).toThrowError(expect.objectContaining({ status: 307, location: "/" }));
  });
});
